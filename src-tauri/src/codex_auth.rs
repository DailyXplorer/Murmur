//! Reads, and refreshes when needed, the ChatGPT session Codex keeps in
//! `$CODEX_HOME/auth.json`.
//!
//! The refresh follows the open-source Codex CLI (`openai/codex`,
//! `codex-rs/login/src/auth/manager.rs`): same token endpoint and client id,
//! same JSON body, same 5-minute expiry window and 8-day fallback interval,
//! and it rewrites only `tokens.id_token`, `tokens.access_token`,
//! `tokens.refresh_token` and `last_refresh`. Every other key in the file is
//! kept. Codex takes no cross-process lock, so Murmur coordinates the way
//! Codex processes do with each other: re-read before refreshing, and keep the
//! other writer's tokens when the file changed underneath.
//!
//! Tokens are never logged.

use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::transcription_error::{FailureKind, TranscriptionError};

const REFRESH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR: &str = "CODEX_REFRESH_TOKEN_URL_OVERRIDE";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ACCESS_TOKEN_REFRESH_WINDOW_MINUTES: i64 = 5;
const TOKEN_REFRESH_INTERVAL_DAYS: i64 = 8;
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);
const PERMANENT_REFRESH_CODES: [&str; 3] = [
    "refresh_token_expired",
    "refresh_token_reused",
    "refresh_token_invalidated",
];

/// One Murmur refresh at a time. The GUI and `--transcribe-file` are separate
/// processes and rely on the re-read checks instead, as Codex processes do.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Hashes of refresh tokens the server rejected for good, so the status check
/// can report "sign in again" without a network call.
static REJECTED_REFRESH_TOKENS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodexAuthStatus {
    /// A session exists and is either valid or refreshable.
    pub signed_in: bool,
    /// A session exists, but it expired and cannot be refreshed.
    pub expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshReason {
    ProactiveExpiry,
    Scheduled,
    UnauthorizedRecovery,
}

impl RefreshReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProactiveExpiry => "proactive_expiry",
            Self::Scheduled => "scheduled",
            Self::UnauthorizedRecovery => "unauthorized_recovery",
        }
    }
}

/// One read of auth.json. `raw` is kept to detect a write by another process.
#[derive(Debug, Clone)]
struct AuthFile {
    raw: String,
    json: Value,
}

impl AuthFile {
    fn read(path: &Path) -> Result<Self, TranscriptionError> {
        // Codex rewrites the file in place (truncate, then write), so a read
        // can land on a half-written file. Retry briefly before giving up.
        let mut last_error = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            let raw = fs::read_to_string(path).map_err(|_| {
                TranscriptionError::new(
                    FailureKind::SignedOut,
                    format!(
                        "Codex/ChatGPT session not found at {}. Open the Codex app and sign in.",
                        path.display()
                    ),
                )
            })?;
            match serde_json::from_str(&raw) {
                Ok(json) => return Ok(Self { raw, json }),
                Err(error) => last_error = Some(error),
            }
        }
        Err(TranscriptionError::new(
            FailureKind::SignedOut,
            format!(
                "failed to parse {}: {}",
                path.display(),
                last_error.map(|e| e.to_string()).unwrap_or_default()
            ),
        ))
    }

    fn token(&self, name: &str) -> Option<&str> {
        self.json
            .get("tokens")?
            .get(name)?
            .as_str()
            .filter(|value| !value.is_empty())
    }

    fn access_token(&self) -> Option<&str> {
        self.token("access_token")
    }

    fn refresh_token(&self) -> Option<&str> {
        self.token("refresh_token")
    }

    fn last_refresh(&self) -> Option<DateTime<Utc>> {
        let raw = self.json.get("last_refresh")?.as_str()?;
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|time| time.with_timezone(&Utc))
    }

    fn session(&self) -> Result<Session, TranscriptionError> {
        let access_token = self.access_token().ok_or_else(|| {
            TranscriptionError::new(
                FailureKind::SignedOut,
                "A ChatGPT subscription session is required. Sign in to Codex with ChatGPT and configure Codex to store credentials in auth.json.",
            )
        })?;
        let account_id = self
            .token("account_id")
            .map(str::to_string)
            .or_else(|| chatgpt_account_id_from_jwt(access_token));
        Ok(Session {
            access_token: access_token.to_string(),
            account_id,
        })
    }

    /// Same triggers as Codex: the access token expires within the window,
    /// or, without an `exp` claim, the last refresh is older than 8 days.
    fn refresh_reason(&self, now: DateTime<Utc>) -> Option<RefreshReason> {
        let access_token = self.access_token()?;
        if let Some(expires_at) = jwt_expiration(access_token) {
            let window = ChronoDuration::minutes(ACCESS_TOKEN_REFRESH_WINDOW_MINUTES);
            return (expires_at <= now + window).then_some(RefreshReason::ProactiveExpiry);
        }
        let last_refresh = self.last_refresh()?;
        (last_refresh < now - ChronoDuration::days(TOKEN_REFRESH_INTERVAL_DAYS))
            .then_some(RefreshReason::Scheduled)
    }

    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.access_token()
            .and_then(jwt_expiration)
            .is_some_and(|expires_at| expires_at <= now)
    }

    fn is_refreshable(&self) -> bool {
        self.refresh_token()
            .is_some_and(|token| !was_rejected(token))
    }

    /// The file content after a refresh: only the returned tokens and
    /// `last_refresh` change. Unknown keys pass through untouched.
    fn with_refreshed_tokens(
        &self,
        tokens: &RefreshResponse,
        now: DateTime<Utc>,
    ) -> Result<String, TranscriptionError> {
        let mut json = self.json.clone();
        let root = json.as_object_mut().ok_or_else(|| {
            TranscriptionError::new(FailureKind::SignedOut, "auth.json is not a JSON object")
        })?;
        let stored = root
            .entry("tokens")
            .or_insert_with(|| Value::Object(Default::default()))
            .as_object_mut()
            .ok_or_else(|| {
                TranscriptionError::new(FailureKind::SignedOut, "auth.json tokens is not an object")
            })?;
        for (name, value) in [
            ("id_token", &tokens.id_token),
            ("access_token", &tokens.access_token),
            ("refresh_token", &tokens.refresh_token),
        ] {
            if let Some(value) = value {
                stored.insert(name.to_string(), Value::String(value.clone()));
            }
        }
        root.insert(
            "last_refresh".to_string(),
            Value::String(now.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
        );
        serde_json::to_string_pretty(&json).map_err(|error| {
            TranscriptionError::new(FailureKind::Other, format!("serialize auth.json: {error}"))
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RefreshResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

enum RefreshFailure {
    /// The refresh token is dead; only a new sign-in helps.
    Permanent(String),
    Transient(TranscriptionError),
}

pub fn auth_path() -> Result<PathBuf, TranscriptionError> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("auth.json"));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        TranscriptionError::new(FailureKind::SignedOut, "User home directory is not set")
    })?;
    Ok(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn refresh_endpoint() -> String {
    std::env::var(REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR)
        .unwrap_or_else(|_| REFRESH_TOKEN_URL.to_string())
}

/// Local view of the session for settings and onboarding. Never refreshes and
/// never touches the network.
pub fn auth_status() -> CodexAuthStatus {
    let Some(file) = auth_path().ok().and_then(|path| AuthFile::read(&path).ok()) else {
        return CodexAuthStatus {
            signed_in: false,
            expired: false,
        };
    };
    status_of(&file, Utc::now())
}

fn status_of(file: &AuthFile, now: DateTime<Utc>) -> CodexAuthStatus {
    if file.access_token().is_none() {
        return CodexAuthStatus {
            signed_in: false,
            expired: false,
        };
    }
    let dead = file.is_expired(now) && !file.is_refreshable();
    CodexAuthStatus {
        signed_in: !dead,
        expired: dead,
    }
}

/// The session to send with the next request, refreshed first when it is
/// about to expire.
pub async fn session_for_request(path: &Path) -> Result<Session, TranscriptionError> {
    session_for_request_at(path, &refresh_endpoint(), Utc::now()).await
}

async fn session_for_request_at(
    path: &Path,
    endpoint: &str,
    now: DateTime<Utc>,
) -> Result<Session, TranscriptionError> {
    let file = AuthFile::read(path)?;
    let Some(reason) = file.refresh_reason(now) else {
        return file.session();
    };
    match refresh(path, endpoint, &file, reason, now).await {
        Ok(session) => Ok(session),
        // Codex keeps using a not-yet-expired token when a proactive refresh
        // fails for a transient reason; the server decides with a 401.
        Err(error) if error.kind != FailureKind::SessionExpired && !file.is_expired(now) => {
            warn!("Proactive Codex token refresh failed; using the current token: {error}");
            file.session()
        }
        Err(error) => Err(error),
    }
}

/// Called once after the transcription endpoint answered 401 for `rejected`.
pub async fn recover_from_unauthorized(
    path: &Path,
    rejected: &Session,
) -> Result<Session, TranscriptionError> {
    let file = AuthFile::read(path)?;
    // Another process may have refreshed since our request left.
    if file.access_token() != Some(rejected.access_token.as_str()) {
        return file.session();
    }
    refresh(
        path,
        &refresh_endpoint(),
        &file,
        RefreshReason::UnauthorizedRecovery,
        Utc::now(),
    )
    .await
}

async fn refresh(
    path: &Path,
    endpoint: &str,
    seen: &AuthFile,
    reason: RefreshReason,
    now: DateTime<Utc>,
) -> Result<Session, TranscriptionError> {
    let _guard = REFRESH_LOCK.lock().await;

    // Guarded reload: if the tokens changed since `seen`, someone else (Codex,
    // or a concurrent Murmur request) refreshed already. Use their result.
    let base = AuthFile::read(path)?;
    if base.access_token() != seen.access_token() {
        info!("Codex session changed on disk; using the refreshed tokens");
        return base.session();
    }

    let refresh_token = base.refresh_token().ok_or_else(|| {
        TranscriptionError::new(
            FailureKind::SessionExpired,
            "The Codex session has no refresh token. Open Codex and sign in again.",
        )
    })?;

    info!("Refreshing the Codex session ({})", reason.as_str());
    let tokens = match request_refresh(endpoint, refresh_token, reason).await {
        Ok(tokens) => tokens,
        Err(RefreshFailure::Transient(error)) => return Err(error),
        Err(RefreshFailure::Permanent(detail)) => {
            // A `refresh_token_reused` answer usually means Codex rotated the
            // token a moment ago; its new tokens are on disk.
            let latest = AuthFile::read(path)?;
            if latest.refresh_token() != Some(refresh_token) {
                return latest.session();
            }
            remember_rejected(refresh_token);
            return Err(TranscriptionError::new(
                FailureKind::SessionExpired,
                format!("Codex session expired: {detail}. Open Codex and sign in again."),
            ));
        }
    };
    let refreshed_access_token = tokens.access_token.clone().ok_or_else(|| {
        TranscriptionError::new(
            FailureKind::SessionExpired,
            "The token refresh returned no access token",
        )
    })?;

    // Re-read right before writing. If anything changed while the request
    // was in flight, the other writer wins and our tokens are discarded.
    let latest = AuthFile::read(path)?;
    if latest.raw != base.raw {
        info!(
            "auth.json changed during the Codex token refresh; keeping the other writer's tokens"
        );
        return latest.session();
    }

    let content = base.with_refreshed_tokens(&tokens, now)?;
    if let Err(error) = write_atomically(path, &content) {
        warn!("Failed to save the refreshed Codex session: {error}");
    } else {
        info!("Saved the refreshed Codex session");
    }

    Ok(Session {
        account_id: base
            .token("account_id")
            .map(str::to_string)
            .or_else(|| chatgpt_account_id_from_jwt(&refreshed_access_token)),
        access_token: refreshed_access_token,
    })
}

async fn request_refresh(
    endpoint: &str,
    refresh_token: &str,
    reason: RefreshReason,
) -> Result<RefreshResponse, RefreshFailure> {
    let client = reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .timeout(REFRESH_TIMEOUT)
        .build()
        .map_err(|error| {
            RefreshFailure::Transient(TranscriptionError::new(
                FailureKind::Other,
                format!("failed to build refresh HTTP client: {error}"),
            ))
        })?;
    let response = client
        .post(endpoint)
        .json(&refresh_request_body(refresh_token, reason))
        .send()
        .await
        .map_err(|error| {
            RefreshFailure::Transient(TranscriptionError::new(
                FailureKind::from_transport(&error),
                format!("token refresh request failed: {error}"),
            ))
        })?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        return serde_json::from_str(&body).map_err(|_| {
            RefreshFailure::Transient(TranscriptionError::new(
                FailureKind::Other,
                "token refresh returned an unreadable response",
            ))
        });
    }
    Err(classify_refresh_failure(status, &body))
}

/// The JSON body Codex sends (`TokenEncoding::Json`, keys sorted).
fn refresh_request_body(
    refresh_token: &str,
    reason: RefreshReason,
) -> BTreeMap<&'static str, &str> {
    BTreeMap::from([
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_reason", reason.as_str()),
        ("refresh_token", refresh_token),
    ])
}

/// Codex's rule: a 401, a known refresh-token error code, or a 400
/// `invalid_grant` is permanent. Anything else may pass.
fn classify_refresh_failure(status: u16, body: &str) -> RefreshFailure {
    let parsed: Option<Value> = serde_json::from_str(body.trim()).ok();
    let code = parsed.as_ref().and_then(|json| {
        let text = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .filter(|code| !code.is_empty())
                .map(str::to_ascii_lowercase)
        };
        text(json.get("error"))
            .or_else(|| text(json.get("error").and_then(|error| error.get("code"))))
            .or_else(|| text(json.get("code")))
    });
    let known_code = code
        .as_deref()
        .is_some_and(|code| PERMANENT_REFRESH_CODES.contains(&code));
    let invalid_grant = status == 400 && code.as_deref() == Some("invalid_grant");
    let detail = code.unwrap_or_else(|| format!("HTTP {status}"));
    if status == 401 || known_code || invalid_grant {
        RefreshFailure::Permanent(detail)
    } else {
        RefreshFailure::Transient(TranscriptionError::new(
            FailureKind::from_status(status),
            format!("token refresh failed: {detail}"),
        ))
    }
}

/// Writes through a temporary file in the same directory, then renames it
/// over the target, so readers never see a partial file. Mode 0600 like Codex.
fn write_atomically(path: &Path, content: &str) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = directory.join(format!(".auth.json.murmur-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn token_fingerprint(token: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

fn remember_rejected(refresh_token: &str) {
    let fingerprint = token_fingerprint(refresh_token);
    if let Ok(mut rejected) = REJECTED_REFRESH_TOKENS.lock() {
        if !rejected.contains(&fingerprint) {
            rejected.push(fingerprint);
        }
    }
}

fn was_rejected(refresh_token: &str) -> bool {
    REJECTED_REFRESH_TOKENS
        .lock()
        .map(|rejected| rejected.contains(&token_fingerprint(refresh_token)))
        .unwrap_or(false)
}

fn jwt_expiration(token: &str) -> Option<DateTime<Utc>> {
    let exp = jwt_payload(token)?.get("exp")?.as_i64()?;
    DateTime::from_timestamp(exp, 0)
}

pub(crate) fn chatgpt_account_id_from_jwt(token: &str) -> Option<String> {
    jwt_payload(token)?
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64url(payload)?;
    serde_json::from_slice(&decoded).ok()
}

fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let padding_start = input.find('=').unwrap_or(input.len());
    let padding = &input[padding_start..];
    let remainder = input.len() % 4;

    if !padding.bytes().all(|byte| byte == b'=') || padding.len() > 2 {
        return None;
    }
    if !padding.is_empty() && remainder != 0 {
        return None;
    }
    if padding.is_empty() && remainder == 1 {
        return None;
    }

    let mut normalized = input.to_string();
    if padding.is_empty() {
        normalized.extend(std::iter::repeat_n('=', (4 - remainder) % 4));
    }

    URL_SAFE.decode(normalized).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::thread;

    fn jwt(claims: Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn auth_json(access_token: &str, refresh_token: &str) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "OPENAI_API_KEY": null,
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "old-id",
                "access_token": access_token,
                "refresh_token": refresh_token,
                "account_id": "acct_1"
            },
            "last_refresh": "2026-10-01T00:00:00Z",
            "agent_identity": {"kept": true}
        }))
        .unwrap()
    }

    fn file_from(raw: String) -> AuthFile {
        AuthFile {
            json: serde_json::from_str(&raw).unwrap(),
            raw,
        }
    }

    /// Serves one HTTP request and hands back the raw request. `before_reply`
    /// runs after the request arrived and before the response is sent.
    fn one_shot_server(
        status: u16,
        body: &'static str,
        before_reply: impl FnOnce() + Send + 'static,
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/oauth/token", listener.local_addr().unwrap());
        let (request_tx, request_rx) = mpsc::channel();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut request_body = vec![0; content_length];
            reader.read_exact(&mut request_body).unwrap();
            head.push_str(&String::from_utf8(request_body).unwrap());
            before_reply();
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let mut stream = stream;
            stream.write_all(response.as_bytes()).unwrap();
            request_tx.send(head).unwrap();
        });
        (endpoint, request_rx)
    }

    fn expiring_token() -> String {
        jwt(serde_json::json!({"exp": now().timestamp() + 60}))
    }

    #[test]
    fn detects_expiry_within_the_codex_window() {
        let soon = file_from(auth_json(&expiring_token(), "rt"));
        assert_eq!(
            soon.refresh_reason(now()),
            Some(RefreshReason::ProactiveExpiry)
        );

        let later = jwt(serde_json::json!({"exp": now().timestamp() + 6 * 60}));
        assert_eq!(
            file_from(auth_json(&later, "rt")).refresh_reason(now()),
            None
        );

        // Without `exp`, fall back to the 8-day interval since last_refresh.
        let opaque = file_from(auth_json("opaque", "rt"));
        assert_eq!(opaque.refresh_reason(now()), Some(RefreshReason::Scheduled));
        let recent = now() - ChronoDuration::days(1);
        let mut json: Value = serde_json::from_str(&auth_json("opaque", "rt")).unwrap();
        json["last_refresh"] = Value::String(recent.to_rfc3339());
        assert_eq!(file_from(json.to_string()).refresh_reason(now()), None);
    }

    #[test]
    fn status_reports_expired_only_without_a_usable_refresh_token() {
        let expired = jwt(serde_json::json!({"exp": now().timestamp() - 1}));
        let refreshable = status_of(&file_from(auth_json(&expired, "rt-ok")), now());
        assert!(refreshable.signed_in && !refreshable.expired);

        let dead = status_of(&file_from(auth_json(&expired, "")), now());
        assert!(!dead.signed_in && dead.expired);

        remember_rejected("rt-rejected");
        let rejected = status_of(&file_from(auth_json(&expired, "rt-rejected")), now());
        assert!(!rejected.signed_in && rejected.expired);

        let valid = jwt(serde_json::json!({"exp": now().timestamp() + 3600}));
        let fine = status_of(&file_from(auth_json(&valid, "")), now());
        assert!(fine.signed_in && !fine.expired);
    }

    #[test]
    fn refresh_sends_the_codex_request_and_rewrites_only_token_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
        let (endpoint, requests) = one_shot_server(
            200,
            r#"{"id_token":"new-id","access_token":"new-access","refresh_token":"rt-new"}"#,
            || {},
        );

        let session =
            tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now()))
                .unwrap();

        assert_eq!(
            session,
            Session {
                access_token: "new-access".to_string(),
                account_id: Some("acct_1".to_string()),
            }
        );
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: application/json\r\n"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
                "grant_type": "refresh_token",
                "refresh_reason": "proactive_expiry",
                "refresh_token": "rt-old"
            })
        );

        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "OPENAI_API_KEY": null,
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": "new-id",
                    "access_token": "new-access",
                    "refresh_token": "rt-new",
                    "account_id": "acct_1"
                },
                "last_refresh": "2026-10-15T12:00:00Z",
                "agent_identity": {"kept": true}
            })
        );
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("auth.json")]);
    }

    #[test]
    fn refresh_keeps_the_other_writers_tokens_when_the_file_changed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
        let theirs = auth_json("codex-access", "codex-rt");
        let path_for_server = path.clone();
        let theirs_for_server = theirs.clone();
        let (endpoint, _requests) = one_shot_server(
            200,
            r#"{"access_token":"murmur-access","refresh_token":"murmur-rt"}"#,
            move || fs::write(&path_for_server, theirs_for_server).unwrap(),
        );

        let session =
            tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now()))
                .unwrap();

        assert_eq!(session.access_token, "codex-access");
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
    }

    #[test]
    fn permanent_refresh_failure_reports_an_expired_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        let expired = jwt(serde_json::json!({"exp": now().timestamp() - 1}));
        let original = auth_json(&expired, "rt-dead");
        fs::write(&path, &original).unwrap();
        let (endpoint, _requests) = one_shot_server(
            400,
            r#"{"error":{"code":"refresh_token_expired","message":"x"}}"#,
            || {},
        );

        let error = tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now()))
            .unwrap_err();

        assert_eq!(error.kind, FailureKind::SessionExpired);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(was_rejected("rt-dead"));
    }

    #[test]
    fn refresh_failures_follow_the_codex_permanence_rule() {
        let permanent = |status, body| {
            matches!(
                classify_refresh_failure(status, body),
                RefreshFailure::Permanent(_)
            )
        };
        assert!(permanent(401, "{}"));
        assert!(permanent(400, r#"{"error":"invalid_grant"}"#));
        assert!(permanent(
            400,
            r#"{"error":{"code":"refresh_token_reused"}}"#
        ));
        assert!(permanent(403, r#"{"code":"refresh_token_invalidated"}"#));
        assert!(!permanent(400, r#"{"error":"invalid_request"}"#));
        assert!(!permanent(500, "oops"));
        match classify_refresh_failure(503, "") {
            RefreshFailure::Transient(error) => assert_eq!(error.kind, FailureKind::ServerError),
            RefreshFailure::Permanent(_) => panic!("503 must be transient"),
        }
    }

    #[test]
    fn extracts_chatgpt_account_id_from_jwt() {
        let token = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xMjMifX0.signature";
        assert_eq!(
            chatgpt_account_id_from_jwt(token).as_deref(),
            Some("acct_123")
        );
    }

    #[test]
    fn decodes_padded_and_unpadded_base64url() {
        assert_eq!(decode_base64url("SGVsbG8"), Some(b"Hello".to_vec()));
        assert_eq!(decode_base64url("SGVsbG8="), Some(b"Hello".to_vec()));
        assert_eq!(decode_base64url("SGk="), Some(b"Hi".to_vec()));
        assert_eq!(decode_base64url("SGk"), Some(b"Hi".to_vec()));
    }

    #[test]
    fn rejects_malformed_base64url_padding() {
        for malformed in [
            "SG=VsbG8",
            "SGVsbG8===",
            "SGVsbG8=A",
            "TQ=",
            "TQ===",
            "TQ==A",
            "A",
            "SGVsbG8==",
        ] {
            assert_eq!(decode_base64url(malformed), None, "accepted {malformed}");
        }
    }

    #[test]
    fn auth_parse_error_names_the_resolved_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(&path, "{").unwrap();
        let error = AuthFile::read(&path).unwrap_err();
        assert!(error.detail.contains(&path.display().to_string()));
        assert_eq!(error.kind, FailureKind::SignedOut);
    }
}
