use anyhow::{anyhow, Context, Result};
use prost::Message;
use security_framework::os::macos::code_signing::{
    Flags as CodeSigningFlags, GuestAttributes, SecCode, SecRequirement,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};
use tonic::client::Grpc;
use tonic::codec::ProstCodec;
use tonic::codegen::http::uri::PathAndQuery;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Response, Status};

const SAMPLE_RATE: usize = 16_000;
const AUDIO_CHUNK_SAMPLES: usize = SAMPLE_RATE;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(600);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(90);
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(5);

const ANTIGRAVITY_APP: &str = "/Applications/Antigravity.app";
const ANTIGRAVITY_BINARY: &str =
    "/Applications/Antigravity.app/Contents/Resources/bin/language_server";
const CODESIGN_PATH: &str = "/usr/bin/codesign";
const ANTIGRAVITY_CODE_REQUIREMENT: &str = r#"anchor apple generic and identifier "language_server" and certificate leaf[subject.OU] = "EQHXZ8M8AV""#;

const START_PATH: &str = "/exa.language_server_pb.LanguageServerService/StreamAudioTranscription";
const SEND_PATH: &str = "/exa.language_server_pb.LanguageServerService/SendAudioChunk";
const END_PATH: &str = "/exa.language_server_pb.LanguageServerService/EndAudioSession";
const CAPABILITIES_PATH: &str = "/exa.language_server_pb.LanguageServerService/GetCapabilities";

/// Reports whether Antigravity and its local session marker are available.
///
/// The marker is inspected through filesystem metadata only; Murmur never
/// opens or copies the Antigravity token.
pub fn status() -> crate::commands::transcription::GeminiStatus {
    crate::commands::transcription::GeminiStatus {
        installed: verified_antigravity_binary().is_ok(),
        // The token is never opened. Its presence only lets the settings page
        // report the session Antigravity has already created. The service
        // performs the authoritative check on the first dictation.
        signed_in: antigravity_token_path().is_some_and(|path| {
            std::fs::metadata(path)
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false)
        }),
    }
}

/// Opens the installed Antigravity app so the user can sign in explicitly.
pub fn open_antigravity() -> Result<()> {
    verified_antigravity_binary()?;

    let status = Command::new("/usr/bin/open")
        .arg(ANTIGRAVITY_APP)
        .status()
        .context("failed to open Antigravity")?;
    if !status.success() {
        return Err(anyhow!("Antigravity could not be opened."));
    }
    Ok(())
}

/// Streams dictation audio through a local Antigravity language server.
pub struct GeminiTranscriber {
    state: Arc<Mutex<RuntimeState>>,
}

impl GeminiTranscriber {
    /// Creates a transcriber with an idle lifecycle supervisor.
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        spawn_supervisor(Arc::downgrade(&state));
        Self { state }
    }

    /// Transcribes normalized mono PCM samples with a Murmur-owned server
    /// started from the verified system Antigravity installation.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let binary = verified_antigravity_binary()?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to initialize Gemini transcription runtime")?;

        // The lock stays held for the request so the idle supervisor cannot
        // stop a Murmur-owned language server while it is transcribing.
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("Gemini transcription state is unavailable"))?;
        state.ensure_connection(&runtime, &binary)?;
        let owned = state
            .owned
            .as_mut()
            .ok_or_else(|| anyhow!("verified Antigravity service is unavailable"))?;
        let result = runtime.block_on(transcribe_over_grpc(owned, samples));
        owned.last_used = Instant::now();
        result.map_err(friendly_transcription_error)
    }

    /// Stops the language server started by this transcriber, if any.
    pub fn shutdown(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.stop_owned();
    }
}

impl Default for GeminiTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct RuntimeState {
    owned: Option<OwnedServer>,
}

impl RuntimeState {
    /// Returns a probed connection to a language server owned by Murmur.
    ///
    /// External services are deliberately never discovered or reused: process
    /// arguments and loopback ownership do not authenticate their identity.
    fn ensure_connection(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        binary: &Path,
    ) -> Result<()> {
        if let Some(owned) = self.owned.as_mut() {
            if owned.has_verified_identity()
                && runtime.block_on(probe_connection(&owned.connection))
                && owned.has_verified_identity()
            {
                return Ok(());
            }

            if let Some(mut stale) = self.owned.take() {
                stale.stop();
            }
        }

        let owned = OwnedServer::start(binary, runtime)?;
        self.owned = Some(owned);
        Ok(())
    }

    fn stop_owned(&mut self) {
        if let Some(mut owned) = self.owned.take() {
            owned.stop();
        }
    }

    /// Stops a Murmur-owned language server after `IDLE_TIMEOUT` without use.
    fn stop_owned_if_idle(&mut self) {
        if self.owned.is_none() {
            return;
        }

        let should_stop = self
            .owned
            .as_ref()
            .is_some_and(|owned| should_stop_owned(owned.last_used, Instant::now()));

        if should_stop {
            self.stop_owned();
        }
    }
}

struct OwnedServer {
    child: Child,
    connection: ConnectionInfo,
    last_used: Instant,
}

impl OwnedServer {
    /// Starts a headless Antigravity language server and waits until it accepts
    /// transcription probes. Omits `--override_ide_version` when the bundle
    /// version cannot be read, instead of substituting Murmur's version.
    fn start(binary: &Path, runtime: &tokio::runtime::Runtime) -> Result<Self> {
        let csrf = generate_csrf_token()?;
        let mut command = Command::new(binary);
        command.args([
            "--headless",
            "--standalone",
            "--disable_telemetry",
            "--override_ide_name=antigravity",
            "--subclient_type=hub",
            "--override_user_agent_name=antigravity",
            "--app_data_dir=antigravity",
            "--api_server_url=https://generativelanguage.googleapis.com",
            "--cloud_code_endpoint=https://daily-cloudcode-pa.googleapis.com",
            "--use_ls_chrome_devtools_mcp=false",
            "--https_server_port=0",
            "--http_server_port=0",
        ]);
        if let Some(version) = antigravity_version(binary) {
            command.arg(format!("--override_ide_version={version}"));
        }
        command
            .arg(format!("--csrf_token={csrf}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start {}", binary.display()))?;
        if let Err(error) = verify_running_antigravity_process(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let connection = ConnectionInfo {
            host: "127.0.0.1".to_string(),
            port: 0,
            csrf,
        };
        let mut owned = Self {
            child,
            connection,
            last_used: Instant::now(),
        };

        let started = Instant::now();
        while started.elapsed() < STARTUP_TIMEOUT {
            if !owned.is_running() {
                return Err(anyhow!(
                    "Antigravity transcription service exited during startup. Open Antigravity, sign in again, and retry."
                ));
            }
            for (host, port) in owned_loopback_endpoints(owned.child.id())? {
                // Re-check the Child handle after lsof and again after the
                // probe. A PID is not an identity once the owned child exits.
                if !owned.has_verified_identity() {
                    return Err(anyhow!(
                        "Antigravity transcription service lost its verified identity during startup. Reinstall Antigravity and retry."
                    ));
                }
                let candidate = ConnectionInfo {
                    host,
                    port,
                    csrf: owned.connection.csrf.clone(),
                };
                if runtime.block_on(probe_connection(&candidate)) && owned.has_verified_identity() {
                    owned.connection = candidate;
                    log::debug!("Started a headless Antigravity transcription service");
                    return Ok(owned);
                }
            }
            thread::sleep(Duration::from_millis(150));
        }

        owned.stop();
        Err(anyhow!(
            "Antigravity transcription service did not become ready. Open Antigravity, sign in again, and retry."
        ))
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn has_verified_identity(&mut self) -> bool {
        match self.verify_identity() {
            Ok(()) => true,
            Err(error) => {
                log::warn!("Rejected Antigravity transcription process identity: {error}");
                false
            }
        }
    }

    fn verify_identity(&mut self) -> Result<()> {
        if !self.is_running() {
            return Err(anyhow!("Antigravity transcription service is not running"));
        }
        verify_running_antigravity_process(self.child.id())
    }

    fn stop(&mut self) {
        if !self.is_running() {
            return;
        }

        let pid = self.child.id().to_string();
        let _ = Command::new("/bin/kill").args(["-INT", &pid]).status();
        for _ in 0..20 {
            if !self.is_running() {
                log::debug!("Stopped Murmur-owned Antigravity transcription service");
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        log::warn!("Forced the Murmur-owned Antigravity transcription service to stop");
    }
}

impl Drop for OwnedServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct ConnectionInfo {
    host: String,
    port: u16,
    csrf: String,
}

/// Periodically stops idle Murmur-owned language servers.
fn spawn_supervisor(state: Weak<Mutex<RuntimeState>>) {
    thread::spawn(move || loop {
        thread::sleep(SUPERVISOR_INTERVAL);
        let Some(state) = state.upgrade() else {
            break;
        };
        if let Ok(mut state) = state.try_lock() {
            if state.owned.is_none() {
                continue;
            }
            state.stop_owned_if_idle();
        };
    });
}

/// Returns the fixed system binary only when its on-disk identity matches the
/// Google-signed Antigravity language server. User-writable install locations,
/// symlinks, ad-hoc signatures, and lookalike identifiers fail closed.
fn verified_antigravity_binary() -> Result<PathBuf> {
    let expected = PathBuf::from(ANTIGRAVITY_BINARY);
    let metadata = std::fs::symlink_metadata(&expected).map_err(|_| {
        anyhow!("Antigravity is not installed. Install it before using Gemini transcription.")
    })?;
    if !metadata.file_type().is_file() {
        return Err(anyhow!(
            "Antigravity's transcription service is not a regular file. Reinstall Antigravity and retry."
        ));
    }

    let canonical = expected
        .canonicalize()
        .context("failed to resolve the Antigravity transcription service")?;
    if canonical != expected {
        return Err(anyhow!(
            "Antigravity's transcription service has an unexpected path. Reinstall Antigravity and retry."
        ));
    }

    verify_antigravity_signature(&canonical)?;
    Ok(canonical)
}

fn codesign_command(binary: &Path) -> Command {
    let mut command = Command::new(CODESIGN_PATH);
    command
        .args(["--verify", "--strict"])
        .arg(format!("-R={ANTIGRAVITY_CODE_REQUIREMENT}"))
        .arg(binary);
    command
}

fn verify_antigravity_signature(binary: &Path) -> Result<()> {
    let output = codesign_command(binary)
        .output()
        .context("failed to verify the Antigravity transcription service")?;
    if !output.status.success() {
        return Err(anyhow!(
            "Antigravity's transcription service failed signature verification. Reinstall Antigravity and retry."
        ));
    }
    Ok(())
}

/// Validates the dynamic code object behind the actual spawned PID. This is
/// the authority check that closes the path verification-to-exec race: even
/// if a user-writable application bundle changes after `codesign` returns,
/// an unsigned or differently signed process is rejected before discovery or
/// reuse of its listener.
fn verify_running_antigravity_process(pid: u32) -> Result<()> {
    let pid = i32::try_from(pid).context("Antigravity process ID is out of range")?;
    let requirement = SecRequirement::from_str(ANTIGRAVITY_CODE_REQUIREMENT)
        .context("failed to compile the Antigravity code requirement")?;
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    let code =
        SecCode::copy_guest_with_attribues(None, &attributes, CodeSigningFlags::NO_NETWORK_ACCESS)
            .context("failed to inspect the running Antigravity transcription service")?;
    code.check_validity(
        CodeSigningFlags::STRICT_VALIDATE | CodeSigningFlags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .context("running Antigravity transcription service failed identity verification")
}

fn antigravity_token_path() -> Option<PathBuf> {
    user_home().map(|home| home.join(".gemini/jetski-standalone-oauth-token"))
}

/// Reads Antigravity's bundle version from Info.plist. Missing or unreadable
/// versions yield `None` so Murmur does not impersonate the IDE version.
fn antigravity_version(binary: &Path) -> Option<String> {
    let contents = binary
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)?;
    let plist = contents.join("Info.plist");
    Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(plist)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

/// True when a Murmur-owned server has been unused for `IDLE_TIMEOUT`.
fn should_stop_owned(last_used: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_used) >= IDLE_TIMEOUT
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn generate_csrf_token() -> Result<String> {
    let output = Command::new("/usr/bin/uuidgen")
        .output()
        .context("failed to generate a Gemini transcription session token")?;
    if !output.status.success() {
        return Err(anyhow!(
            "failed to generate a Gemini transcription session token"
        ));
    }
    let token = String::from_utf8(output.stdout)
        .context("Gemini transcription session token was not UTF-8")?
        .trim()
        .to_ascii_lowercase();
    if token.is_empty() {
        return Err(anyhow!(
            "failed to generate a Gemini transcription session token"
        ));
    }
    Ok(token)
}

fn owned_loopback_endpoints(pid: u32) -> Result<Vec<(String, u16)>> {
    let endpoints = listening_tcp_endpoints(pid)?;
    if endpoints
        .iter()
        .any(|(host, _)| normalize_loopback_host(host).is_none())
    {
        return Err(anyhow!(
            "Antigravity transcription service attempted to listen outside loopback"
        ));
    }
    Ok(endpoints
        .into_iter()
        .filter_map(|(host, port)| normalize_loopback_host(&host).map(|host| (host, port)))
        .collect())
}

fn listening_tcp_endpoints(pid: u32) -> Result<Vec<(String, u16)>> {
    let output = Command::new("/usr/sbin/lsof")
        .args([
            "-nP",
            "-a",
            "-p",
            &pid.to_string(),
            "-iTCP",
            "-sTCP:LISTEN",
            "-Fn",
        ])
        .output()
        .context("failed to inspect Antigravity listening ports")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut endpoints = BTreeSet::new();
    for line in stdout.lines().filter(|line| line.starts_with('n')) {
        let address = &line[1..];
        if let Some((host, port)) = address.rsplit_once(':') {
            if let Ok(port) = port.parse::<u16>() {
                endpoints.insert((host.to_string(), port));
            }
        }
    }
    Ok(endpoints.into_iter().collect())
}

/// Maps recognized loopback listener hosts to a gRPC endpoint hostname.
fn normalize_loopback_host(host: &str) -> Option<String> {
    match host {
        "127.0.0.1" | "localhost" | "[::1]" => Some(host.to_string()),
        "::1" => Some("[::1]".to_string()),
        "::ffff:127.0.0.1" => Some("[::ffff:127.0.0.1]".to_string()),
        "[::ffff:127.0.0.1]" => Some(host.to_string()),
        _ => None,
    }
}

async fn connect(connection: &ConnectionInfo) -> Result<Channel> {
    Endpoint::from_shared(format!("http://{}:{}", connection.host, connection.port))
        .context("invalid local Antigravity endpoint")?
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TRANSCRIPTION_TIMEOUT)
        .connect()
        .await
        .context("failed to connect to the local Antigravity service")
}

fn authenticated_request<T>(message: T, csrf: &str) -> Result<Request<T>> {
    let mut request = Request::new(message);
    let metadata =
        MetadataValue::try_from(csrf).context("invalid local Antigravity session metadata")?;
    request
        .metadata_mut()
        .insert("x-codeium-csrf-token", metadata);
    Ok(request)
}

async fn probe_connection(connection: &ConnectionInfo) -> bool {
    tokio::time::timeout(PROBE_TIMEOUT, probe_connection_inner(connection))
        .await
        .unwrap_or(false)
}

async fn probe_connection_inner(connection: &ConnectionInfo) -> bool {
    let Ok(channel) = connect(connection).await else {
        return false;
    };
    let mut grpc = Grpc::new(channel);
    if grpc.ready().await.is_err() {
        return false;
    }
    let Ok(request) = authenticated_request(GetCapabilitiesRequest {}, &connection.csrf) else {
        return false;
    };
    let response: std::result::Result<Response<GetCapabilitiesResponse>, Status> = grpc
        .unary(
            request,
            PathAndQuery::from_static(CAPABILITIES_PATH),
            ProstCodec::default(),
        )
        .await;
    response.is_ok()
}

/// Bounds a full Gemini transcription attempt with `TRANSCRIPTION_TIMEOUT`.
async fn transcribe_over_grpc(owned: &mut OwnedServer, samples: &[f32]) -> Result<String> {
    tokio::time::timeout(
        TRANSCRIPTION_TIMEOUT,
        transcribe_over_grpc_inner(owned, samples),
    )
    .await
    .map_err(|_| anyhow!("Gemini transcription timed out"))?
}

/// Sends audio, ends the session, and reads the completed transcript.
async fn transcribe_over_grpc_inner(owned: &mut OwnedServer, samples: &[f32]) -> Result<String> {
    owned.verify_identity()?;
    let connection = owned.connection.clone();
    let channel = connect(&connection).await?;
    // If the owned process died while the TCP transport was opening, never
    // send microphone data to a listener that may have replaced its port.
    owned.verify_identity()?;
    let mut start_client = Grpc::new(channel.clone());
    start_client
        .ready()
        .await
        .context("local Antigravity transcription service is not ready")?;
    let start_request = authenticated_request(
        StartAudioTranscriptionRequest {
            mime_type: "audio/pcm;rate=16000".to_string(),
            model: String::new(),
            pre_cursor_text: String::new(),
            post_cursor_text: String::new(),
            cascade_id: String::new(),
            continuous: false,
        },
        &connection.csrf,
    )?;
    let response: Response<tonic::codec::Streaming<StreamAudioTranscriptionResponse>> =
        start_client
            .server_streaming(
                start_request,
                PathAndQuery::from_static(START_PATH),
                ProstCodec::default(),
            )
            .await
            .context("failed to start Gemini audio transcription")?;
    let mut stream = response.into_inner();

    let session_id = loop {
        let message = stream
            .message()
            .await
            .context("failed to read Gemini transcription stream")?
            .ok_or_else(|| anyhow!("Gemini transcription stream closed before it was ready"))?;
        if let Some(stream_audio_transcription_response::Message::Ready(ready)) = message.message {
            if !ready.session_id.is_empty() {
                break ready.session_id;
            }
        }
    };

    let send_result: Result<()> = async {
        for (sequence, chunk) in samples.chunks(AUDIO_CHUNK_SAMPLES).enumerate() {
            owned.verify_identity()?;
            let sequence_number = i32::try_from(sequence)
                .context("Gemini transcription audio is too long to sequence")?;
            send_audio_chunk(
                channel.clone(),
                &connection,
                &session_id,
                sequence_number,
                chunk,
            )
            .await?;
            owned.verify_identity()?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = send_result {
        match tokio::time::timeout(
            PROBE_TIMEOUT,
            end_audio_session(channel, &connection, &session_id),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(end_error)) => log::warn!(
                "Failed to end Gemini audio session after an audio send error: {end_error}"
            ),
            Err(_) => {
                log::warn!("Timed out while ending Gemini audio session after an audio send error")
            }
        }
        return Err(error);
    }
    owned.verify_identity()?;
    end_audio_session(channel, &connection, &session_id).await?;

    let mut transcript = String::new();
    let mut saw_complete = false;
    while let Some(message) = stream
        .message()
        .await
        .context("failed to read Gemini transcription result")?
    {
        match message.message {
            Some(stream_audio_transcription_response::Message::Transcription(update)) => {
                if !update.text.is_empty() {
                    transcript = update.text;
                }
            }
            Some(stream_audio_transcription_response::Message::Complete(_)) => {
                saw_complete = true;
                break;
            }
            _ => {}
        }
    }

    if !saw_complete {
        return Err(anyhow!(
            "Gemini transcription stream closed before completion"
        ));
    }
    Ok(transcript.trim().to_string())
}

/// Sends one sequenced PCM chunk to the active Gemini audio session.
async fn send_audio_chunk(
    channel: Channel,
    connection: &ConnectionInfo,
    session_id: &str,
    sequence_number: i32,
    samples: &[f32],
) -> Result<()> {
    let data = pcm_le_bytes(samples);

    let request = authenticated_request(
        SendAudioChunkRequest {
            session_id: session_id.to_string(),
            data,
            sequence_number,
        },
        &connection.csrf,
    )?;
    let mut client = Grpc::new(channel);
    client
        .ready()
        .await
        .context("local Antigravity service stopped while sending audio")?;
    let _: Response<SendAudioChunkResponse> = client
        .unary(
            request,
            PathAndQuery::from_static(SEND_PATH),
            ProstCodec::default(),
        )
        .await
        .context("failed to send audio to Gemini transcription")?;
    Ok(())
}

fn pcm_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut data = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        data.extend_from_slice(&value.to_le_bytes());
    }
    data
}

/// Closes the Gemini audio session so the service can emit the final transcript.
async fn end_audio_session(
    channel: Channel,
    connection: &ConnectionInfo,
    session_id: &str,
) -> Result<()> {
    let request = authenticated_request(
        EndAudioSessionRequest {
            session_id: session_id.to_string(),
        },
        &connection.csrf,
    )?;
    let mut client = Grpc::new(channel);
    client
        .ready()
        .await
        .context("local Antigravity service stopped before transcription completed")?;
    let _: Response<EndAudioSessionResponse> = client
        .unary(
            request,
            PathAndQuery::from_static(END_PATH),
            ProstCodec::default(),
        )
        .await
        .context("failed to finish Gemini audio transcription")?;
    Ok(())
}

fn friendly_transcription_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(status) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Status>())
    {
        let message = status.message().to_ascii_lowercase();
        if matches!(
            status.code(),
            Code::Unauthenticated | Code::PermissionDenied
        ) || message.contains("authentication")
            || message.contains("sign in")
        {
            return anyhow!(
                "Gemini transcription session expired. Open Antigravity, sign in again, and retry."
            );
        }
        if message.contains("no available audio transcription models") {
            return anyhow!(
                "Gemini transcription is unavailable in this Antigravity session. Open Antigravity, check the session, and retry."
            );
        }
    }
    error
}

#[derive(Clone, PartialEq, Message)]
struct StartAudioTranscriptionRequest {
    #[prost(string, tag = "1")]
    mime_type: String,
    #[prost(string, tag = "2")]
    model: String,
    #[prost(string, tag = "3")]
    pre_cursor_text: String,
    #[prost(string, tag = "4")]
    post_cursor_text: String,
    #[prost(string, tag = "5")]
    cascade_id: String,
    #[prost(bool, tag = "6")]
    continuous: bool,
}

#[derive(Clone, PartialEq, Message)]
struct StreamAudioTranscriptionResponse {
    #[prost(
        oneof = "stream_audio_transcription_response::Message",
        tags = "1, 2, 3"
    )]
    message: Option<stream_audio_transcription_response::Message>,
}

mod stream_audio_transcription_response {
    use super::{AudioStreamComplete, AudioStreamReady, TranscriptionUpdate};
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Message {
        #[prost(message, tag = "1")]
        Ready(AudioStreamReady),
        #[prost(message, tag = "2")]
        Transcription(TranscriptionUpdate),
        #[prost(message, tag = "3")]
        Complete(AudioStreamComplete),
    }
}

#[derive(Clone, PartialEq, Message)]
struct AudioStreamReady {
    #[prost(string, tag = "1")]
    session_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct TranscriptionUpdate {
    #[prost(string, tag = "1")]
    text: String,
    #[prost(bool, tag = "2")]
    is_final: bool,
}

#[derive(Clone, PartialEq, Message)]
struct AudioStreamComplete {}

#[derive(Clone, PartialEq, Message)]
struct SendAudioChunkRequest {
    #[prost(string, tag = "1")]
    session_id: String,
    #[prost(bytes = "vec", tag = "2")]
    data: Vec<u8>,
    #[prost(int32, tag = "3")]
    sequence_number: i32,
}

#[derive(Clone, PartialEq, Message)]
struct SendAudioChunkResponse {}

#[derive(Clone, PartialEq, Message)]
struct EndAudioSessionRequest {
    #[prost(string, tag = "1")]
    session_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct EndAudioSessionResponse {}

#[derive(Clone, PartialEq, Message)]
struct GetCapabilitiesRequest {}

#[derive(Clone, PartialEq, Message)]
struct GetCapabilitiesResponse {
    #[prost(bool, tag = "1")]
    supports_hook_result_proto_bytes: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_verification_is_pinned_to_google_language_server() {
        let command = codesign_command(Path::new(ANTIGRAVITY_BINARY));
        assert_eq!(command.get_program(), CODESIGN_PATH);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "--verify".to_string(),
                "--strict".to_string(),
                format!("-R={ANTIGRAVITY_CODE_REQUIREMENT}"),
                ANTIGRAVITY_BINARY.to_string(),
            ]
        );
        assert!(ANTIGRAVITY_CODE_REQUIREMENT.contains("identifier \"language_server\""));
        assert!(ANTIGRAVITY_CODE_REQUIREMENT.contains("EQHXZ8M8AV"));
    }

    #[test]
    fn rejects_unsigned_antigravity_lookalike() {
        let file = tempfile::NamedTempFile::new().expect("temporary file should be created");
        assert!(verify_antigravity_signature(file.path()).is_err());
    }

    #[test]
    fn rejects_running_process_with_the_wrong_code_identity() {
        assert!(verify_running_antigravity_process(std::process::id()).is_err());
    }

    #[test]
    fn pcm_chunk_uses_little_endian_i16() {
        let bytes = pcm_le_bytes(&[-2.0_f32, -1.0, 0.0, 1.0, 2.0]);
        assert_eq!(bytes, vec![1, 128, 1, 128, 0, 0, 255, 127, 255, 127]);
    }

    /// Stops a Murmur-owned server only after the idle deadline elapses.
    #[test]
    fn owned_server_stops_at_idle_deadline() {
        let last_used = Instant::now();
        assert!(!should_stop_owned(
            last_used,
            last_used + IDLE_TIMEOUT - Duration::from_millis(1)
        ));
        assert!(should_stop_owned(last_used, last_used + IDLE_TIMEOUT));
    }

    /// Accepts IPv4, IPv6, and IPv4-mapped loopback hosts for gRPC endpoints.
    #[test]
    fn accepts_only_loopback_listener_hosts() {
        assert_eq!(
            normalize_loopback_host("127.0.0.1").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            normalize_loopback_host("localhost").as_deref(),
            Some("localhost")
        );
        assert_eq!(normalize_loopback_host("[::1]").as_deref(), Some("[::1]"));
        assert_eq!(normalize_loopback_host("::1").as_deref(), Some("[::1]"));
        assert_eq!(
            normalize_loopback_host("::ffff:127.0.0.1").as_deref(),
            Some("[::ffff:127.0.0.1]")
        );
        assert_eq!(
            normalize_loopback_host("[::ffff:127.0.0.1]").as_deref(),
            Some("[::ffff:127.0.0.1]")
        );
        assert_eq!(normalize_loopback_host("*"), None);
        assert_eq!(normalize_loopback_host("0.0.0.0"), None);
        assert_eq!(normalize_loopback_host("192.168.1.10"), None);
    }

    /// Leaves the IDE version unset when the Antigravity bundle cannot be read.
    #[test]
    fn missing_antigravity_bundle_version_has_no_murmur_fallback() {
        assert_eq!(antigravity_version(Path::new("language_server")), None);
    }

    #[test]
    #[ignore]
    fn live_antigravity_session_transcribes_wav() {
        let path = std::env::var("MURMUR_GEMINI_TEST_WAV")
            .expect("MURMUR_GEMINI_TEST_WAV must point to a 16 kHz mono PCM WAV");
        let mut reader = hound::WavReader::open(path).expect("test WAV should open");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, SAMPLE_RATE as u32);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        let samples = reader
            .samples::<i16>()
            .map(|sample| sample.expect("test WAV sample should decode") as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();

        let transcriber = GeminiTranscriber::new();
        let transcript = transcriber
            .transcribe(&samples)
            .expect("live Gemini transcription should succeed");
        transcriber.shutdown();
        assert!(
            transcriber
                .state
                .lock()
                .expect("live test state should remain available")
                .owned
                .is_none(),
            "shutdown should release the owned server before a headless exit"
        );
        assert!(!transcript.is_empty());
        println!("Gemini transcript: {transcript}");
    }
}
