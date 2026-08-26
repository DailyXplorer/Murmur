use anyhow::{anyhow, Context, Result};
use prost::Message;
use std::collections::BTreeSet;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(90);
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(5);

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
        available_on_platform: true,
        installed: antigravity_binary().is_some(),
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
    if antigravity_binary().is_none() {
        return Err(anyhow!(
            "Antigravity is not installed. Install it before using Gemini transcription."
        ));
    }

    Command::new("/usr/bin/open")
        .args(["-a", "Antigravity"])
        .spawn()
        .context("failed to open Antigravity")?;
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

    /// Transcribes normalized mono PCM samples with the active Antigravity
    /// session, borrowing an existing server or starting a managed one.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let binary = antigravity_binary().ok_or_else(|| {
            anyhow!("Antigravity is not installed. Install it before using Gemini transcription.")
        })?;

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
        let connection = state.connection(&runtime, &binary)?;
        let result = runtime.block_on(transcribe_over_grpc(&connection, samples));
        state.mark_used();
        result.map_err(friendly_transcription_error)
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
    fn connection(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        binary: &Path,
    ) -> Result<ConnectionInfo> {
        let external_process = external_server_process_exists(binary);
        if external_process {
            let started = Instant::now();
            while started.elapsed() < STARTUP_TIMEOUT {
                for candidate in discover_external_connections(binary)? {
                    if runtime.block_on(probe_connection(&candidate)) {
                        if let Some(mut owned) = self.owned.take() {
                            owned.stop();
                        }
                        log::debug!(
                            "Using the language server managed by Antigravity for Gemini transcription"
                        );
                        return Ok(candidate);
                    }
                }
                thread::sleep(Duration::from_millis(150));
            }

            return Err(anyhow!(
                "Antigravity is running, but its transcription service is not ready. Wait a moment and retry."
            ));
        }

        if let Some(owned) = self.owned.as_mut() {
            if owned.is_running() && runtime.block_on(probe_connection(&owned.connection)) {
                return Ok(owned.connection.clone());
            }

            let mut stale = self.owned.take().expect("owned server was present");
            stale.stop();
        }

        let owned = OwnedServer::start(binary, runtime)?;
        let connection = owned.connection.clone();
        self.owned = Some(owned);
        Ok(connection)
    }

    fn mark_used(&mut self) {
        if let Some(owned) = self.owned.as_mut() {
            owned.last_used = Instant::now();
        }
    }

    fn stop_owned_if_idle_or_superseded(&mut self, binary: Option<&Path>) {
        if self.owned.is_none() {
            return;
        }

        let external_server_exists = binary.is_some_and(external_server_process_exists);
        let should_stop = self.owned.as_ref().is_some_and(|owned| {
            should_stop_owned(owned.last_used, Instant::now(), external_server_exists)
        });

        if should_stop {
            if let Some(mut owned) = self.owned.take() {
                owned.stop();
            }
        }
    }
}

struct OwnedServer {
    child: Child,
    connection: ConnectionInfo,
    last_used: Instant,
}

impl OwnedServer {
    fn start(binary: &Path, runtime: &tokio::runtime::Runtime) -> Result<Self> {
        let port = reserve_loopback_port()?;
        let csrf = generate_csrf_token()?;
        let version = antigravity_version(binary);
        let mut command = Command::new(binary);
        command
            .args([
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
            ])
            .arg(format!("--override_ide_version={version}"))
            .arg(format!("--http_server_port={port}"))
            .arg(format!("--csrf_token={csrf}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command
            .spawn()
            .with_context(|| format!("failed to start {}", binary.display()))?;
        let connection = ConnectionInfo { port, csrf };
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
            if runtime.block_on(probe_connection(&owned.connection)) {
                log::debug!("Started a headless Antigravity transcription service");
                return Ok(owned);
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
    port: u16,
    csrf: String,
}

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
            let binary = antigravity_binary();
            state.stop_owned_if_idle_or_superseded(binary.as_deref());
        };
    });
}

fn antigravity_binary() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(
        "/Applications/Antigravity.app/Contents/Resources/bin/language_server",
    )];
    if let Some(home) = user_home() {
        candidates
            .push(home.join("Applications/Antigravity.app/Contents/Resources/bin/language_server"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn antigravity_token_path() -> Option<PathBuf> {
    user_home().map(|home| home.join(".gemini/jetski-standalone-oauth-token"))
}

fn antigravity_version(binary: &Path) -> String {
    let Some(contents) = binary
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return env!("CARGO_PKG_VERSION").to_string();
    };
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
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

fn should_stop_owned(last_used: Instant, now: Instant, external_server_exists: bool) -> bool {
    external_server_exists || now.saturating_duration_since(last_used) >= IDLE_TIMEOUT
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn reserve_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .context("failed to reserve a loopback port for Gemini transcription")?;
    let port = listener
        .local_addr()
        .context("failed to read reserved Gemini transcription port")?
        .port();
    drop(listener);
    Ok(port)
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

fn external_server_process_exists(binary: &Path) -> bool {
    language_server_processes(binary)
        .map(|processes| !processes.is_empty())
        .unwrap_or(false)
}

fn discover_external_connections(binary: &Path) -> Result<Vec<ConnectionInfo>> {
    let mut connections = Vec::new();
    for process in language_server_processes(binary)? {
        for port in listening_ports(process.pid)? {
            connections.push(ConnectionInfo {
                port,
                csrf: process.csrf.clone(),
            });
        }
    }
    Ok(connections)
}

struct ExternalProcess {
    pid: u32,
    csrf: String,
}

fn language_server_processes(binary: &Path) -> Result<Vec<ExternalProcess>> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
        .context("failed to inspect Antigravity processes")?;
    if !output.status.success() {
        return Err(anyhow!("failed to inspect Antigravity processes"));
    }

    let expected_binary = binary.to_string_lossy();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in stdout.lines() {
        let mut fields = line.split_whitespace();
        let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let args = fields.collect::<Vec<_>>();
        if args.first().copied() != Some(expected_binary.as_ref())
            || args.contains(&"--headless")
            || !has_flag_value(&args, "--override_ide_name", "antigravity")
        {
            continue;
        }
        if let Some(csrf) = flag_value(&args, "--csrf_token") {
            processes.push(ExternalProcess { pid, csrf });
        }
    }
    Ok(processes)
}

fn flag_value(args: &[&str], flag: &str) -> Option<String> {
    for (index, argument) in args.iter().enumerate() {
        if *argument == flag {
            return args.get(index + 1).map(|value| (*value).to_string());
        }
        if let Some(value) = argument.strip_prefix(&format!("{flag}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn has_flag_value(args: &[&str], flag: &str, expected: &str) -> bool {
    flag_value(args, flag).as_deref() == Some(expected)
}

fn listening_ports(pid: u32) -> Result<Vec<u16>> {
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
    let mut ports = BTreeSet::new();
    for line in stdout.lines().filter(|line| line.starts_with('n')) {
        let address = &line[1..];
        if !(address.starts_with("127.0.0.1:") || address.starts_with("localhost:")) {
            continue;
        }
        if let Some(port) = address
            .rsplit(':')
            .next()
            .and_then(|value| value.parse::<u16>().ok())
        {
            ports.insert(port);
        }
    }
    Ok(ports.into_iter().collect())
}

async fn connect(connection: &ConnectionInfo) -> Result<Channel> {
    Endpoint::from_shared(format!("http://127.0.0.1:{}", connection.port))
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

async fn transcribe_over_grpc(connection: &ConnectionInfo, samples: &[f32]) -> Result<String> {
    tokio::time::timeout(
        TRANSCRIPTION_TIMEOUT,
        transcribe_over_grpc_inner(connection, samples),
    )
    .await
    .map_err(|_| anyhow!("Gemini transcription timed out"))?
}

async fn transcribe_over_grpc_inner(
    connection: &ConnectionInfo,
    samples: &[f32],
) -> Result<String> {
    let channel = connect(connection).await?;
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

    for (sequence, chunk) in samples.chunks(AUDIO_CHUNK_SAMPLES).enumerate() {
        let sequence_number = i32::try_from(sequence)
            .context("Gemini transcription audio is too long to sequence")?;
        send_audio_chunk(
            channel.clone(),
            connection,
            &session_id,
            sequence_number,
            chunk,
        )
        .await?;
    }
    end_audio_session(channel, connection, &session_id).await?;

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

async fn send_audio_chunk(
    channel: Channel,
    connection: &ConnectionInfo,
    session_id: &str,
    sequence_number: i32,
    samples: &[f32],
) -> Result<()> {
    let mut data = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        data.extend_from_slice(&value.to_le_bytes());
    }

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
    fn parses_split_and_equals_flags() {
        let split = ["--csrf_token", "secret"];
        let equals = ["--csrf_token=secret"];
        assert_eq!(
            flag_value(&split, "--csrf_token").as_deref(),
            Some("secret")
        );
        assert_eq!(
            flag_value(&equals, "--csrf_token").as_deref(),
            Some("secret")
        );
    }

    #[test]
    fn pcm_chunk_uses_little_endian_i16() {
        let samples = [-1.0_f32, 0.0, 1.0];
        let mut bytes = Vec::new();
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(bytes, vec![1, 128, 0, 0, 255, 127]);
    }

    #[test]
    fn owned_server_stops_at_idle_deadline_or_when_antigravity_opens() {
        let last_used = Instant::now();
        assert!(!should_stop_owned(
            last_used,
            last_used + IDLE_TIMEOUT - Duration::from_millis(1),
            false
        ));
        assert!(should_stop_owned(
            last_used,
            last_used + IDLE_TIMEOUT,
            false
        ));
        assert!(should_stop_owned(last_used, last_used, true));
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

        let transcript = GeminiTranscriber::new()
            .transcribe(&samples)
            .expect("live Gemini transcription should succeed");
        assert!(!transcript.is_empty());
        println!("Gemini transcript: {transcript}");
    }
}
