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
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener as StdListener;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::thread;

    fn jwt(claims: Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    fn id_token() -> String {
        jwt(json!({
            "email": "someone@example.com",
            "https://api.openai.com/auth": {"chatgpt_account_id": "acct_9"}
        }))
    }

    fn tokens_reply() -> (u16, String) {
        (
            200,
            json!({"id_token": id_token(), "access_token": "at-1", "refresh_token": "rt-1"})
                .to_string(),
        )
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
            request.push_str(&line);
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        request.push_str(&String::from_utf8(body).unwrap());
        request
    }

    /// A fake issuer: answers each request with the next reply, in order, and
    /// hands back the raw requests.
    fn fake_issuer(replies: Vec<(u16, String)>) -> (String, mpsc::Receiver<String>) {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let (requests_tx, requests_rx) = mpsc::channel();
        thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let request = read_http_request(&mut stream);
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                let _ = requests_tx.send(request);
            }
        });
        (issuer, requests_rx)
    }

    /// An issuer nothing listens on.
    fn unreachable_issuer() -> String {
        format!("http://127.0.0.1:{}", free_port())
    }

    fn free_port() -> u16 {
        StdListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// One GET from the browser; the raw answer back.
    fn browse(url: &str) -> String {
        let url = Url::parse(url).unwrap();
        let (host, port) = (url.host_str().unwrap(), url.port().unwrap());
        let mut stream = std::net::TcpStream::connect((host, port)).unwrap();
        let target = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        };
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn param(url: &str, name: &str) -> Option<String> {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }

    fn header(response: &str, name: &str) -> Option<String> {
        response.split("\r\n\r\n").next()?.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }

    /// Where the provider sends the browser back, with `query` appended.
    fn callback(authorize: &str, query: &str) -> String {
        browse(&format!(
            "{}?{query}",
            param(authorize, "redirect_uri").unwrap()
        ))
    }

    fn with_state(authorize: &str, query: &str) -> String {
        let state = param(authorize, "state").unwrap();
        let encoded: String = url::form_urlencoded::byte_serialize(state.as_bytes()).collect();
        format!("{query}&state={encoded}")
    }

    /// The browser after a successful sign-in: the callback's answer, then
    /// the page it redirects to.
    fn finish_in_browser(authorize: &str) -> (String, String) {
        let answer = callback(authorize, &with_state(authorize, "code=the-code"));
        let page = header(&answer, "location")
            .map(|location| browse(&location))
            .unwrap_or_default();
        (answer, page)
    }

    fn form_of(request: &str) -> BTreeMap<String, String> {
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    fn config(issuer: &str, directory: &Path) -> LoginConfig {
        LoginConfig {
            issuer: issuer.to_string(),
            ports: vec![0],
            timeout: Duration::from_secs(20),
            auth_path: directory.join("codex").join("auth.json"),
        }
    }

    /// Runs the sign-in while `browser` plays the user's browser on the URL
    /// Murmur opened, and returns both outcomes.
    async fn sign_in_with<T: Send + 'static>(
        config: &LoginConfig,
        browser: impl FnOnce(String) -> T + Send + 'static,
    ) -> (Result<(), SignInError>, T) {
        let (handle_tx, handle_rx) = mpsc::channel();
        let result = sign_in(
            config,
            move |url| {
                let url = url.to_string();
                handle_tx.send(thread::spawn(move || browser(url))).unwrap();
                Ok(())
            },
            std::future::pending(),
        )
        .await;
        (result, handle_rx.recv().unwrap().join().unwrap())
    }

    fn redirect_port(authorize: &str) -> u16 {
        Url::parse(&param(authorize, "redirect_uri").unwrap())
            .unwrap()
            .port()
            .unwrap()
    }

    /// Asks this process, not the system: a child another test spawns at the
    /// same moment gets a copy of every open socket, and on macOS keeps the
    /// port taken for a while after the sign-in closed it. Windows has no
    /// `/dev/fd` to ask; the shared sign-in code is checked on macOS.
    fn assert_port_is_free(port: u16) {
        #[cfg(unix)]
        assert_eq!(sockets_on(port), 0, "the sign-in let go of its port");
        #[cfg(windows)]
        let _ = port;
    }

    #[cfg(unix)]
    fn sockets_on(port: u16) -> usize {
        fs::read_dir("/dev/fd")
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .filter(|&fd| local_port(fd) == Some(port))
            .count()
    }

    #[cfg(unix)]
    fn local_port(fd: std::os::fd::RawFd) -> Option<u16> {
        // SAFETY: an all-zero sockaddr_storage is a valid value.
        let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&address) as libc::socklen_t;
        // SAFETY: `address` and `length` are alive for the call and `length`
        // is the size of `address`; a closed or non-socket fd only errors.
        let named =
            unsafe { libc::getsockname(fd, (&raw mut address).cast(), &raw mut length) } == 0;
        (named && i32::from(address.ss_family) == libc::AF_INET).then(|| {
            // SAFETY: the family says `address` holds a sockaddr_in.
            let address = unsafe { &*(&raw const address).cast::<libc::sockaddr_in>() };
            u16::from_be(address.sin_port)
        })
    }

    #[test]
    fn pkce_and_state_are_random_url_safe_and_match() {
        let pkce = Pkce::new();
        assert_eq!(pkce.verifier.len(), 86);
        assert!(pkce
            .verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
        assert_eq!(
            pkce.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()))
        );
        assert_ne!(Pkce::new().verifier, pkce.verifier);

        let state = random_b64(32);
        assert_eq!(state.len(), 43);
        assert_ne!(random_b64(32), state);
    }

    #[test]
    fn the_authorize_url_is_the_one_codex_opens() {
        let pkce = Pkce {
            verifier: "unused".to_string(),
            challenge: "chal-lenge_1".to_string(),
        };
        assert_eq!(
            authorize_url(
                "https://auth.openai.com",
                "http://127.0.0.1:1455/auth/callback",
                &pkce,
                "st/ate"
            ),
            "https://auth.openai.com/oauth/authorize?response_type=code\
             &client_id=app_EMoamEEZ73f0CkXaXp7hrann\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback\
             &code_challenge=chal-lenge_1\
             &code_challenge_method=S256\
             &state=st%2Fate\
             &scope=openid+profile+email+offline_access+api.connectors.read+api.connectors.invoke\
             &id_token_add_organizations=true\
             &codex_cli_simplified_flow=true\
             &originator=codex_cli_rs"
        );
    }

    #[tokio::test]
    async fn signs_in_like_codex_and_saves_the_shared_session() {
        let directory = tempfile::tempdir().unwrap();
        let (issuer, requests) = fake_issuer(vec![
            tokens_reply(),
            (200, json!({"access_token": "sk-key"}).to_string()),
        ]);
        let config = config(&issuer, directory.path());
        fs::create_dir_all(config.auth_path.parent().unwrap()).unwrap();
        fs::write(
            &config.auth_path,
            json!({
                "OPENAI_API_KEY": "sk-old",
                "auth_mode": "apikey",
                "tokens": {"access_token": "old", "extra": 1},
                "last_refresh": "2020-01-01T00:00:00Z",
                "agent_identity": {"id": 1},
                "personal_access_token": "pat",
                "bedrock_api_key": "bk",
                "bedrock_access_keys": {"k": "v"},
                "future_key": {"kept": true}
            })
            .to_string(),
        )
        .unwrap();

        let (result, (authorize, cancel, (answer, page))) = sign_in_with(&config, |authorize| {
            let cancel = browse(
                &param(&authorize, "redirect_uri")
                    .unwrap()
                    .replace(CALLBACK_PATH, "/cancel"),
            );
            let finished = finish_in_browser(&authorize);
            (authorize, cancel, finished)
        })
        .await;

        assert_eq!(result, Ok(()));
        let port = redirect_port(&authorize);
        assert_eq!(
            param(&authorize, "redirect_uri").unwrap(),
            format!("http://127.0.0.1:{port}/auth/callback")
        );
        assert!(cancel.starts_with("HTTP/1.1 404"), "{cancel}");
        assert!(answer.starts_with("HTTP/1.1 302"), "{answer}");
        assert_eq!(
            header(&answer, "location").unwrap(),
            format!("http://127.0.0.1:{port}/success")
        );
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(
            page.contains("Signed in to ChatGPT. You can close this tab and go back to Murmur.")
        );
        assert!(!page.contains("someone@example.com") && !page.contains("at-1"));

        let token_request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(token_request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
        assert!(token_request
            .to_ascii_lowercase()
            .contains("content-type: application/x-www-form-urlencoded\r\n"));
        let form = form_of(&token_request);
        assert_eq!(
            form.keys().collect::<Vec<_>>(),
            [
                "client_id",
                "code",
                "code_verifier",
                "grant_type",
                "redirect_uri"
            ]
        );
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "the-code");
        assert_eq!(form["client_id"], CLIENT_ID);
        assert_eq!(
            form["redirect_uri"],
            param(&authorize, "redirect_uri").unwrap()
        );
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
            param(&authorize, "code_challenge").unwrap()
        );

        let key_request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(key_request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
        assert_eq!(
            form_of(&key_request),
            BTreeMap::from(
                [
                    ("client_id", CLIENT_ID.to_string()),
                    (
                        "grant_type",
                        "urn:ietf:params:oauth:grant-type:token-exchange".to_string()
                    ),
                    ("requested_token", "openai-api-key".to_string()),
                    ("subject_token", id_token()),
                    (
                        "subject_token_type",
                        "urn:ietf:params:oauth:token-type:id_token".to_string()
                    ),
                ]
                .map(|(key, value)| (key.to_string(), value))
            )
        );

        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&config.auth_path).unwrap()).unwrap();
        let owner = saved["tokens"]["login_owner_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(owner.len(), 43);
        let last_refresh = saved["last_refresh"].as_str().unwrap().to_string();
        let saved_at = chrono::DateTime::parse_from_rfc3339(&last_refresh).unwrap();
        assert!(
            (Utc::now() - saved_at.with_timezone(&Utc))
                .num_seconds()
                .abs()
                < 60
        );
        assert_eq!(
            saved,
            json!({
                "OPENAI_API_KEY": "sk-key",
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": id_token(),
                    "access_token": "at-1",
                    "refresh_token": "rt-1",
                    "account_id": "acct_9",
                    "oauth_provenance": {"issuer": issuer, "client_id": CLIENT_ID},
                    "login_owner_id": owner
                },
                "last_refresh": last_refresh,
                "future_key": {"kept": true}
            })
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&config.auth_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let leftovers: Vec<_> = fs::read_dir(config.auth_path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("auth.json")]);
        assert_port_is_free(port);
    }

    #[tokio::test]
    async fn a_wrong_state_is_refused_and_the_sign_in_goes_on() {
        let directory = tempfile::tempdir().unwrap();
        let (issuer, _requests) = fake_issuer(vec![tokens_reply(), (500, "{}".to_string())]);
        let config = config(&issuer, directory.path());
        let path = config.auth_path.clone();

        let (result, (wrong, missing, saved_before, (answer, _))) =
            sign_in_with(&config, move |authorize| {
                let wrong = callback(&authorize, "code=the-code&state=forged");
                let missing = callback(&authorize, "code=the-code");
                let saved_before = path.exists();
                (wrong, missing, saved_before, finish_in_browser(&authorize))
            })
            .await;

        assert!(wrong.starts_with("HTTP/1.1 400"), "{wrong}");
        assert!(wrong.ends_with("State mismatch"), "{wrong}");
        assert!(missing.starts_with("HTTP/1.1 400"), "{missing}");
        assert!(!saved_before);
        assert!(answer.starts_with("HTTP/1.1 302"), "{answer}");
        assert_eq!(result, Ok(()));
        assert!(config.auth_path.exists());
    }

    #[tokio::test]
    async fn a_refusal_in_the_browser_writes_nothing() {
        for (query, expected) in [
            ("error=access_denied", SignInError::Denied),
            (
                "error=access_denied&error_description=Missing_Codex_Entitlement%20for%20workspace",
                SignInError::NotEntitled,
            ),
            (
                "error=server_error&error_description=missing_codex_entitlement",
                SignInError::Denied,
            ),
            ("", SignInError::Rejected),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let config = config(&unreachable_issuer(), directory.path());

            let (result, (authorize, answer)) = sign_in_with(&config, move |authorize| {
                let answer = callback(&authorize, &with_state(&authorize, query));
                (authorize, answer)
            })
            .await;

            assert_eq!(result, Err(expected), "{query}");
            assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
            assert!(answer.contains("Murmur"), "{answer}");
            assert!(!config.auth_path.exists(), "{query}");
            assert_port_is_free(redirect_port(&authorize));
        }
    }

    #[tokio::test]
    async fn a_failed_code_exchange_writes_nothing() {
        for (issuer, expected) in [
            (
                fake_issuer(vec![(500, "{}".to_string())]).0,
                SignInError::Rejected,
            ),
            (
                fake_issuer(vec![(200, "{\"access_token\":\"at\"}".to_string())]).0,
                SignInError::Rejected,
            ),
            (unreachable_issuer(), SignInError::Network),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let config = config(&issuer, directory.path());

            let (result, (answer, _)) =
                sign_in_with(&config, |authorize| finish_in_browser(&authorize)).await;

            assert_eq!(result, Err(expected), "{issuer}");
            assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
            assert!(!config.auth_path.exists());
        }
    }

    #[tokio::test]
    async fn a_failed_api_key_exchange_still_saves_the_session() {
        let directory = tempfile::tempdir().unwrap();
        let (issuer, _requests) = fake_issuer(vec![tokens_reply(), (500, "{}".to_string())]);
        let config = config(&issuer, directory.path());

        let (result, _) = sign_in_with(&config, |authorize| finish_in_browser(&authorize)).await;

        assert_eq!(result, Ok(()));
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&config.auth_path).unwrap()).unwrap();
        assert_eq!(saved["OPENAI_API_KEY"], Value::Null);
        assert_eq!(saved["auth_mode"], "chatgpt");
        assert_eq!(saved["tokens"]["refresh_token"], "rt-1");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(config.auth_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[tokio::test]
    async fn a_sign_in_without_a_session_id_token_claim_saves_a_null_account() {
        let directory = tempfile::tempdir().unwrap();
        let reply =
            json!({"id_token": jwt(json!({})), "access_token": "at", "refresh_token": "rt"});
        let (issuer, _requests) =
            fake_issuer(vec![(200, reply.to_string()), (500, "{}".to_string())]);
        let config = config(&issuer, directory.path());
        fs::create_dir_all(config.auth_path.parent().unwrap()).unwrap();
        fs::write(&config.auth_path, "[1, 2]").unwrap();

        let (result, _) = sign_in_with(&config, |authorize| finish_in_browser(&authorize)).await;

        assert_eq!(result, Ok(()));
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&config.auth_path).unwrap()).unwrap();
        assert_eq!(saved["tokens"]["account_id"], Value::Null);
    }

    #[tokio::test]
    async fn a_timeout_or_a_cancel_ends_the_sign_in_and_frees_the_port() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = config(&unreachable_issuer(), directory.path());
        config.timeout = Duration::from_millis(300);
        let (opened_tx, opened_rx) = mpsc::channel::<String>();
        let opened = opened_tx.clone();
        let result = sign_in(
            &config,
            move |url| {
                opened.send(url.to_string()).unwrap();
                Ok(())
            },
            std::future::pending(),
        )
        .await;
        assert_eq!(result, Err(SignInError::TimedOut));
        assert_port_is_free(redirect_port(&opened_rx.recv().unwrap()));

        config.timeout = Duration::from_secs(20);
        let result = sign_in(
            &config,
            move |url| {
                opened_tx.send(url.to_string()).unwrap();
                Ok(())
            },
            tokio::time::sleep(Duration::from_millis(300)),
        )
        .await;
        assert_eq!(result, Err(SignInError::Cancelled));
        assert_port_is_free(redirect_port(&opened_rx.recv().unwrap()));
        assert!(!config.auth_path.exists());
    }

    #[tokio::test]
    async fn busy_ports_are_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let holders = [
            StdListener::bind("127.0.0.1:0").unwrap(),
            StdListener::bind("127.0.0.1:0").unwrap(),
        ];
        let ports: Vec<u16> = holders
            .iter()
            .map(|holder| {
                holder.set_nonblocking(true).unwrap();
                holder.local_addr().unwrap().port()
            })
            .collect();
        let mut config = config(&unreachable_issuer(), directory.path());
        config.ports = ports.clone();

        let result = sign_in(
            &config,
            |_: &str| -> Result<(), String> { panic!("no browser without a port") },
            std::future::pending(),
        )
        .await;

        assert_eq!(result, Err(SignInError::PortBusy));
        for holder in &holders {
            assert_eq!(
                holder.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "a holder was contacted"
            );
        }

        let (issuer, _requests) = fake_issuer(vec![tokens_reply(), (500, "{}".to_string())]);
        config.issuer = issuer;
        config.ports = vec![ports[0], 0];
        let (result, (authorize, _)) = sign_in_with(&config, |authorize| {
            let finished = finish_in_browser(&authorize);
            (authorize, finished)
        })
        .await;
        assert_eq!(result, Ok(()));
        assert_ne!(redirect_port(&authorize), ports[0]);
        assert_eq!(
            holders[0].accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[tokio::test]
    async fn a_new_sign_in_cancels_the_one_under_way_and_takes_its_port() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(&unreachable_issuer(), directory.path());
        let slot = SignInSlot::new();
        let (first_opened_tx, first_opened_rx) = oneshot::channel::<String>();
        let (second_opened_tx, second_opened_rx) = oneshot::channel();

        let first = slot.run(|cancelled| {
            sign_in(
                &config,
                move |url| {
                    first_opened_tx.send(url.to_string()).unwrap();
                    Ok(())
                },
                async move {
                    let _ = cancelled.await;
                },
            )
        });
        let second = async {
            let port = redirect_port(&first_opened_rx.await.unwrap());
            let config = LoginConfig {
                issuer: config.issuer.clone(),
                ports: vec![port, 0],
                timeout: config.timeout,
                auth_path: config.auth_path.clone(),
            };
            let run = slot.run(|cancelled| {
                sign_in(
                    &config,
                    move |url| {
                        let taken = redirect_port(url) == port;
                        second_opened_tx.send((taken, sockets_on(port))).unwrap();
                        Ok(())
                    },
                    async move {
                        let _ = cancelled.await;
                    },
                )
            });
            let cancel = async {
                let opened = second_opened_rx.await.unwrap();
                slot.cancel();
                opened
            };
            tokio::join!(run, cancel)
        };
        let (first, (second, (taken, sockets))) = tokio::join!(first, second);

        assert_eq!(first, Err(SignInError::Cancelled));
        assert_eq!(second, Err(SignInError::Cancelled));
        // Only a copy a child of another test holds for a moment makes the
        // second fall back; the first sign-in is gone either way.
        assert_eq!(sockets, usize::from(taken), "the first let go of its port");
    }

    fn session_file(directory: &Path, tokens: Value) -> LoginConfig {
        let config = config(&unreachable_issuer(), directory);
        fs::create_dir_all(config.auth_path.parent().unwrap()).unwrap();
        fs::write(
            &config.auth_path,
            json!({"auth_mode": "chatgpt", "OPENAI_API_KEY": null, "tokens": tokens}).to_string(),
        )
        .unwrap();
        config
    }

    #[tokio::test]
    async fn signing_out_revokes_like_codex_and_deletes_the_file() {
        for (tokens, expected) in [
            (
                json!({"access_token": "at-1", "refresh_token": "rt-1"}),
                json!({"token": "rt-1", "token_type_hint": "refresh_token", "client_id": CLIENT_ID}),
            ),
            (
                json!({"access_token": "at-1", "refresh_token": ""}),
                json!({"token": "at-1", "token_type_hint": "access_token"}),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut config = session_file(directory.path(), tokens);
            let (issuer, requests) = fake_issuer(vec![(200, "{}".to_string())]);
            config.issuer = issuer;

            assert_eq!(sign_out(&config).await, Ok(()));

            let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(request.starts_with("POST /oauth/revoke HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains("content-type: application/json\r\n"));
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body, expected);
            assert!(!config.auth_path.exists());
        }
    }

    #[tokio::test]
    async fn signing_out_deletes_the_file_even_when_the_revoke_fails() {
        for issuer in [
            fake_issuer(vec![(500, "{}".to_string())]).0,
            unreachable_issuer(),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut config = session_file(directory.path(), json!({"refresh_token": "rt-1"}));
            config.issuer = issuer;

            assert_eq!(sign_out(&config).await, Ok(()));
            assert!(!config.auth_path.exists());
        }
    }

    #[tokio::test]
    async fn signing_out_without_a_session_is_fine() {
        let directory = tempfile::tempdir().unwrap();
        let (issuer, requests) = fake_issuer(vec![(200, "{}".to_string())]);
        let config = config(&issuer, directory.path());

        assert_eq!(sign_out(&config).await, Ok(()));
        assert!(requests.recv_timeout(Duration::from_millis(200)).is_err());
    }

    /// Prints the real authorize URL, for a manual check that OpenAI accepts
    /// it. Binds the real ports, cancels at once, and writes nothing.
    #[tokio::test]
    #[ignore = "binds the real ports and prints a real authorize URL"]
    async fn prints_the_real_authorize_url() {
        let directory = tempfile::tempdir().unwrap();
        let config = LoginConfig::new(directory.path().join("auth.json"));

        let result = sign_in(
            &config,
            |url| {
                println!("{url}");
                Ok(())
            },
            async {},
        )
        .await;

        assert_eq!(result, Err(SignInError::Cancelled));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
