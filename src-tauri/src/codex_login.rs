//! Signs in to ChatGPT in the browser the way the Codex CLI does
//! (`openai/codex`, `codex-rs/login/src/server.rs`), and signs out the way
//! `codex logout` does. The session goes to the auth.json Codex reads, so
//! Murmur and Codex share it: signing in or out in one does it for both.
//!
//! Tokens, codes, verifiers and states are never logged.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{SecondsFormat, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::net::Ipv4Addr;
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use url::Url;

use crate::codex_auth::{chatgpt_account_id_from_jwt, write_atomically, CLIENT_ID, REFRESH_LOCK};

pub(crate) const ISSUER: &str = "https://auth.openai.com";
/// Codex's default port and the fallback its redirect allow-list also has.
const PORTS: [u16; 2] = [1455, 1457];
pub(crate) const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const ORIGINATOR: &str = "codex_cli_rs";
const CALLBACK_PATH: &str = "/auth/callback";
const SUCCESS_PATH: &str = "/success";
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const REVOKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the success page stays available after the session is saved.
const SUCCESS_PAGE_WAIT: Duration = Duration::from_secs(10);
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Keys Codex's own sign-in writes as absent.
const CLEARED_KEYS: [&str; 4] = [
    "agent_identity",
    "personal_access_token",
    "bedrock_api_key",
    "bedrock_access_keys",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SignInError {
    PortBusy,
    Cancelled,
    TimedOut,
    Denied,
    NotEntitled,
    Network,
    Rejected,
    SaveFailed,
    BrowserFailed,
}

pub(crate) struct LoginConfig {
    issuer: String,
    ports: Vec<u16>,
    timeout: Duration,
    auth_path: PathBuf,
}

impl LoginConfig {
    pub(crate) fn new(auth_path: PathBuf) -> Self {
        Self {
            issuer: ISSUER.to_string(),
            ports: PORTS.to_vec(),
            timeout: SIGN_IN_TIMEOUT,
            auth_path,
        }
    }
}

struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn new() -> Self {
        let verifier = random_b64(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

fn random_b64(bytes: usize) -> String {
    let mut buffer = vec![0; bytes];
    // As in Codex, whose generator panics too: a sign-in without
    // unpredictable values must not go ahead.
    getrandom::fill(&mut buffer).expect("the system random number generator is available");
    URL_SAFE_NO_PAD.encode(buffer)
}

fn authorize_url(issuer: &str, redirect_uri: &str, pkce: &Pkce, state: &str) -> String {
    let base = format!("{}/oauth/authorize", issuer.trim_end_matches('/'));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect_uri),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("scope", SCOPES),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", ORIGINATOR),
        ])
        .finish();
    format!("{base}?{query}")
}

/// Signs in through the browser `open` shows the authorize page in, and
/// saves the session to `config.auth_path`. Ends early on `cancelled` or
/// after `config.timeout`. The port is free again whatever the outcome.
pub(crate) async fn sign_in(
    config: &LoginConfig,
    open: impl FnOnce(&str) -> Result<(), String>,
    cancelled: impl Future<Output = ()>,
) -> Result<(), SignInError> {
    let listener = bind(&config.ports).await.ok_or(SignInError::PortBusy)?;
    let port = listener
        .local_addr()
        .map_err(|_| SignInError::PortBusy)?
        .port();
    let flow = Flow {
        config,
        redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
        success_url: format!("http://127.0.0.1:{port}{SUCCESS_PATH}"),
        pkce: Pkce::new(),
        state: random_b64(32),
    };
    // The opener's error can quote the URL, and with it the state.
    if open(&authorize_url(
        &config.issuer,
        &flow.redirect_uri,
        &flow.pkce,
        &flow.state,
    ))
    .is_err()
    {
        warn!("Couldn't open the browser for the ChatGPT sign-in");
        return Err(SignInError::BrowserFailed);
    }
    info!("Waiting for the ChatGPT sign-in in the browser, on port {port}");

    let mut server = Server::new(listener);
    tokio::pin!(cancelled);
    let outcome = tokio::select! {
        outcome = flow.await_callback(&mut server) => outcome,
        () = &mut cancelled => Err(SignInError::Cancelled),
        () = tokio::time::sleep(config.timeout) => Err(SignInError::TimedOut),
    };
    match outcome {
        Ok(()) => {
            info!("Signed in to ChatGPT");
            tokio::select! {
                () = serve_success_page(&mut server) => {}
                () = &mut cancelled => {}
                () = tokio::time::sleep(SUCCESS_PAGE_WAIT) => {}
            }
        }
        Err(error) => info!("The ChatGPT sign-in ended: {error:?}"),
    }
    outcome
}

/// The first of `ports` that is free. Unlike Codex, nothing is asked to give
/// its port up: the holder may be a Codex sign-in under way.
async fn bind(ports: &[u16]) -> Option<TcpListener> {
    for &port in ports {
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => return Some(listener),
            Err(error) => info!("Port {port} is not free for the ChatGPT sign-in: {error}"),
        }
    }
    None
}

struct Flow<'a> {
    config: &'a LoginConfig,
    redirect_uri: String,
    success_url: String,
    pkce: Pkce,
    state: String,
}

impl Flow<'_> {
    /// Serves until a callback with the right state arrives, and answers it.
    /// Callbacks with another state are refused and the wait goes on, as in
    /// Codex.
    async fn await_callback(&self, server: &mut Server) -> Result<(), SignInError> {
        loop {
            let (stream, request) = server.next().await;
            let Request::Get(url) = request else {
                respond(stream, "400 Bad Request", "", "Bad Request").await;
                continue;
            };
            if url.path() != CALLBACK_PATH {
                respond(stream, "404 Not Found", "", "Not Found").await;
                continue;
            }
            let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
            if params.get("state") != Some(&self.state) {
                respond(stream, "400 Bad Request", "", "State mismatch").await;
                continue;
            }
            let outcome = self.complete(&params).await;
            match outcome {
                Ok(()) => {
                    let location = format!("Location: {}\r\n", self.success_url);
                    respond(stream, "302 Found", &location, "").await;
                }
                Err(error) => respond_html(stream, browser_message(error)).await,
            }
            return outcome;
        }
    }

    async fn complete(&self, params: &HashMap<String, String>) -> Result<(), SignInError> {
        if let Some(error) = params.get("error") {
            let description = params
                .get("error_description")
                .map(|description| description.to_ascii_lowercase())
                .unwrap_or_default();
            if error == "access_denied" && description.contains("missing_codex_entitlement") {
                return Err(SignInError::NotEntitled);
            }
            return Err(SignInError::Denied);
        }
        let code = params
            .get("code")
            .filter(|code| !code.is_empty())
            .ok_or(SignInError::Rejected)?;

        let client = http_client(TOKEN_TIMEOUT).map_err(|_| SignInError::Network)?;
        let endpoint = format!("{}/oauth/token", self.config.issuer.trim_end_matches('/'));
        let body = post_form(
            &client,
            &endpoint,
            &[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("client_id", CLIENT_ID),
                ("code_verifier", self.pkce.verifier.as_str()),
            ],
        )
        .await?;
        let tokens: TokenResponse = serde_json::from_str(&body).map_err(|_| {
            warn!("The ChatGPT token endpoint returned an unreadable answer");
            SignInError::Rejected
        })?;

        let api_key = post_form(
            &client,
            &endpoint,
            &[
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:token-exchange",
                ),
                ("client_id", CLIENT_ID),
                ("requested_token", "openai-api-key"),
                ("subject_token", tokens.id_token.as_str()),
                (
                    "subject_token_type",
                    "urn:ietf:params:oauth:token-type:id_token",
                ),
            ],
        )
        .await
        .ok()
        .and_then(|body| serde_json::from_str::<ApiKeyResponse>(&body).ok())
        .map(|key| key.access_token);

        save(
            &self.config.auth_path,
            &self.config.issuer,
            &tokens,
            api_key,
        )
        .await
        .map_err(|error| {
            warn!("Couldn't save the ChatGPT session: {error}");
            SignInError::SaveFailed
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct ApiKeyResponse {
    access_token: String,
}

fn http_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .timeout(timeout)
        .build()
}

/// The answer's body, or why there is none.
async fn post_form(
    client: &reqwest::Client,
    endpoint: &str,
    pairs: &[(&str, &str)],
) -> Result<String, SignInError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let response = client
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|error| {
            warn!("The ChatGPT token request failed: {error}");
            SignInError::Network
        })?;
    let status = response.status();
    if !status.is_success() {
        warn!("The ChatGPT token endpoint answered {status}");
        return Err(SignInError::Rejected);
    }
    response.text().await.map_err(|_| SignInError::Rejected)
}

/// Writes the session as Codex's sign-in does. Top-level keys Codex does not
/// know are kept, since another version of Codex may have written them.
async fn save(
    path: &Path,
    issuer: &str,
    tokens: &TokenResponse,
    api_key: Option<String>,
) -> std::io::Result<()> {
    let _guard = REFRESH_LOCK.lock().await;
    let mut root = fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|json| match json {
            Value::Object(root) => Some(root),
            _ => None,
        })
        .unwrap_or_default();
    for key in CLEARED_KEYS {
        root.remove(key);
    }
    root.insert("auth_mode".to_string(), json!("chatgpt"));
    root.insert("OPENAI_API_KEY".to_string(), json!(api_key));
    root.insert(
        "tokens".to_string(),
        json!({
            "id_token": tokens.id_token,
            "access_token": tokens.access_token,
            "refresh_token": tokens.refresh_token,
            "account_id": chatgpt_account_id_from_jwt(&tokens.id_token),
            "oauth_provenance": {"issuer": issuer, "client_id": CLIENT_ID},
            "login_owner_id": random_b64(32),
        }),
    );
    root.insert(
        "last_refresh".to_string(),
        json!(Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true)),
    );
    let content =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(std::io::Error::other)?;
    // Mode 0700 like Codex; on Windows the folder keeps the access rules of
    // the user's profile folder.
    if let Some(directory) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(directory)?;
    }
    write_atomically(path, &content)
}

/// What the browser tab says when the sign-in fails there.
fn browser_message(error: SignInError) -> &'static str {
    match error {
        SignInError::NotEntitled => {
            "Codex isn't enabled for this ChatGPT workspace. Ask your workspace admin, then go back to Murmur."
        }
        SignInError::Denied => {
            "Sign-in was cancelled. You can close this tab and go back to Murmur."
        }
        _ => "Sign-in couldn't be completed. You can close this tab and go back to Murmur to try again.",
    }
}

const SUCCESS_MESSAGE: &str = "Signed in to ChatGPT. You can close this tab and go back to Murmur.";

async fn serve_success_page(server: &mut Server) {
    loop {
        let (stream, request) = server.next().await;
        match request {
            Request::Get(url) if url.path() == SUCCESS_PATH => {
                respond_html(stream, SUCCESS_MESSAGE).await;
                return;
            }
            Request::Get(_) => respond(stream, "404 Not Found", "", "Not Found").await,
            Request::Malformed => respond(stream, "400 Bad Request", "", "Bad Request").await,
        }
    }
}

enum Request {
    Get(Url),
    Malformed,
}

/// The loopback server. Connections are read side by side, so a browser's
/// idle preconnect cannot hold up the callback. Dropping it closes the port
/// and every connection still open.
struct Server {
    listener: TcpListener,
    reading: JoinSet<Option<(TcpStream, Request)>>,
}

impl Server {
    fn new(listener: TcpListener) -> Self {
        Self {
            listener,
            reading: JoinSet::new(),
        }
    }

    async fn next(&mut self) -> (TcpStream, Request) {
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        self.reading.spawn(read_request(stream));
                    }
                }
                Some(read) = self.reading.join_next(), if !self.reading.is_empty() => {
                    if let Ok(Some(request)) = read {
                        return request;
                    }
                }
            }
        }
    }
}

/// The request line of one HTTP request; `None` when the connection sends
/// nothing usable in time.
async fn read_request(mut stream: TcpStream) -> Option<(TcpStream, Request)> {
    let mut head = Vec::new();
    let read = async {
        let mut chunk = [0; 1024];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            if head.len() > MAX_REQUEST_BYTES {
                return false;
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return false,
                Ok(count) => head.extend_from_slice(&chunk[..count]),
            }
        }
        true
    };
    if !tokio::time::timeout(REQUEST_READ_TIMEOUT, read)
        .await
        .unwrap_or(false)
    {
        return None;
    }
    let line = String::from_utf8_lossy(&head);
    let mut parts = line.lines().next().unwrap_or_default().split(' ');
    let request = match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => {
            Url::parse(&format!("http://127.0.0.1{target}"))
                .map(Request::Get)
                .unwrap_or(Request::Malformed)
        }
        _ => Request::Malformed,
    };
    Some((stream, request))
}

async fn respond(mut stream: TcpStream, status: &str, headers: &str, body: &str) {
    respond_with(
        &mut stream,
        status,
        headers,
        "text/plain; charset=utf-8",
        body,
    )
    .await;
}

async fn respond_html(mut stream: TcpStream, message: &str) {
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Murmur</title><style>\
         body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
         font:16px/1.5 -apple-system,BlinkMacSystemFont,sans-serif;color:#1d1d1f;background:#fff}}\
         @media (prefers-color-scheme:dark){{body{{color:#f5f5f7;background:#1d1d1f}}}}\
         p{{max-width:28rem;padding:0 1.5rem;text-align:center}}\
         </style></head><body><p>{message}</p></body></html>"
    );
    respond_with(&mut stream, "200 OK", "", "text/html; charset=utf-8", &page).await;
}

async fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    headers: &str,
    content_type: &str,
    body: &str,
) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Signs out as `codex logout` does: revokes the refresh token, or else the
/// access token, at best effort, then deletes the whole file.
pub(crate) async fn sign_out(config: &LoginConfig) -> Result<(), String> {
    let _guard = REFRESH_LOCK.lock().await;
    let raw = match fs::read_to_string(&config.auth_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        raw => raw.ok(),
    };
    if let Some(body) = raw.as_deref().and_then(revoke_body) {
        let endpoint = format!("{}/oauth/revoke", config.issuer.trim_end_matches('/'));
        let revoked = async {
            http_client(REVOKE_TIMEOUT)?
                .post(&endpoint)
                .json(&body)
                .send()
                .await?
                .error_for_status()
        }
        .await;
        if let Err(error) = revoked {
            warn!("Couldn't revoke the ChatGPT session; signing out anyway: {error}");
        }
    }
    match fs::remove_file(&config.auth_path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(format!(
            "Couldn't remove {}: {error}",
            config.auth_path.display()
        )),
        _ => {
            info!("Signed out of ChatGPT");
            Ok(())
        }
    }
}

fn revoke_body(raw: &str) -> Option<Value> {
    let json: Value = serde_json::from_str(raw).ok()?;
    let token = |name: &str| {
        json.get("tokens")
            .and_then(|tokens| tokens.get(name))
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
    };
    if let Some(refresh_token) = token("refresh_token") {
        return Some(json!({
            "token": refresh_token,
            "token_type_hint": "refresh_token",
            "client_id": CLIENT_ID,
        }));
    }
    token("access_token")
        .map(|access_token| json!({"token": access_token, "token_type_hint": "access_token"}))
}

struct Running {
    cancel: oneshot::Sender<()>,
    ended: oneshot::Receiver<()>,
}

/// One sign-in at a time in this process. A new one cancels the one under
/// way and waits until it let go of its port.
pub(crate) struct SignInSlot(Mutex<Option<Running>>);

impl SignInSlot {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Runs `flow`, which ends when the receiver it gets fires.
    pub(crate) async fn run<T, F, Fut>(&self, flow: F) -> T
    where
        F: FnOnce(oneshot::Receiver<()>) -> Fut,
        Fut: Future<Output = T>,
    {
        let (cancel, cancelled) = oneshot::channel();
        let (_ended, ended) = oneshot::channel();
        let previous = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(Running { cancel, ended });
        if let Some(previous) = previous {
            let _ = previous.cancel.send(());
            let _ = previous.ended.await;
        }
        flow(cancelled).await
    }

    pub(crate) fn cancel(&self) {
        let running = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(running) = running {
            let _ = running.cancel.send(());
        }
    }
}

#[cfg(test)]
mod tests;
