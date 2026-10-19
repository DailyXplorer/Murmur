//! "Test connection" on the Debug page: sends one small request to
//! chatgpt.com per client identity and transport, and reports which ones
//! Cloudflare let through. Only the status and a few response headers are
//! logged, never the token or a body.

use log::info;
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use specta::Type;
use std::time::Duration;

use crate::codex_auth::{self, Session};
use crate::codex_transcribe::{pcm_f32_to_wav_bytes, TRANSCRIBE_URL};

const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const CHROMIUM_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Type)]
pub struct ProbeResult {
    pub name: String,
    /// `<status> <content-type> cf-mitigated=<value>`, or the transport error.
    pub outcome: String,
    /// Cloudflare forwarded the request: the answer is not its HTML challenge.
    pub passed: bool,
}

#[derive(Clone, Copy)]
enum Transport {
    /// Murmur's client: reqwest's default TLS, SChannel on Windows.
    Default,
    NativeTlsHttp1,
    #[cfg(windows)]
    RustlsHttp1,
}

struct Variant {
    name: &'static str,
    url: &'static str,
    transport: Transport,
    user_agent: String,
    originator: &'static str,
    authenticated: bool,
}

fn variants() -> Vec<Variant> {
    // What Murmur sent on Windows before it took the Codex CLI's identity.
    let legacy = format!("Murmur/{} (Mac OS; x64)", env!("CARGO_PKG_VERSION"));
    #[cfg(windows)]
    let cli = crate::codex_transcribe::user_agent();
    #[cfg(not(windows))]
    let cli = crate::codex_transcribe::codex_cli_user_agent("10.0.26100", std::env::consts::ARCH);
    let chromium = CHROMIUM_USER_AGENT.to_string();
    let (desktop, cli_originator) = ("Codex Desktop", "codex_cli_rs");
    let variant = |name, url, transport, user_agent: &String, originator, authenticated| Variant {
        name,
        url,
        transport,
        user_agent: user_agent.clone(),
        originator,
        authenticated,
    };
    #[allow(unused_mut)]
    let mut variants = vec![
        variant(
            "murmur-legacy",
            TRANSCRIBE_URL,
            Transport::Default,
            &legacy,
            desktop,
            false,
        ),
        variant(
            "murmur-legacy-auth",
            TRANSCRIBE_URL,
            Transport::Default,
            &legacy,
            desktop,
            true,
        ),
        variant(
            "codex-cli",
            TRANSCRIBE_URL,
            Transport::Default,
            &cli,
            cli_originator,
            false,
        ),
        variant(
            "codex-cli-auth",
            TRANSCRIBE_URL,
            Transport::Default,
            &cli,
            cli_originator,
            true,
        ),
        variant(
            "native-tls-http1",
            TRANSCRIBE_URL,
            Transport::NativeTlsHttp1,
            &legacy,
            desktop,
            false,
        ),
        variant(
            "chromium-ua",
            TRANSCRIBE_URL,
            Transport::Default,
            &chromium,
            desktop,
            false,
        ),
        variant(
            "codex-cli-auth-responses",
            RESPONSES_URL,
            Transport::Default,
            &cli,
            cli_originator,
            true,
        ),
    ];
    #[cfg(windows)]
    variants.insert(
        4,
        variant(
            "rustls-http1",
            TRANSCRIBE_URL,
            Transport::RustlsHttp1,
            &legacy,
            desktop,
            false,
        ),
    );
    variants
}

/// Runs every variant one after the other; a session is used when one exists.
pub async fn run() -> Vec<ProbeResult> {
    let session = match codex_auth::auth_path() {
        Ok(path) => codex_auth::session_for_request(&path).await.ok(),
        Err(_) => None,
    };
    let silent_wav = pcm_f32_to_wav_bytes(&[0.0; 1_600]).unwrap_or_default();
    let mut results = Vec::new();
    for variant in variants() {
        let result = match (&session, variant.authenticated) {
            (None, true) => ProbeResult {
                name: variant.name.to_string(),
                outcome: "skipped: no Codex session".to_string(),
                passed: false,
            },
            (session, _) => probe(&variant, session.as_ref(), &silent_wav).await,
        };
        info!("probe {}: {}", result.name, result.outcome);
        results.push(result);
    }
    results
}

async fn probe(variant: &Variant, session: Option<&Session>, wav: &[u8]) -> ProbeResult {
    let result = |outcome, passed| ProbeResult {
        name: variant.name.to_string(),
        outcome,
        passed,
    };
    let builder = reqwest::Client::builder()
        .user_agent(&variant.user_agent)
        .timeout(Duration::from_secs(20));
    let builder = match variant.transport {
        Transport::Default => builder,
        Transport::NativeTlsHttp1 => builder.http1_only(),
        #[cfg(windows)]
        Transport::RustlsHttp1 => builder.use_rustls_tls().http1_only(),
    };
    let client = match builder.build() {
        Ok(client) => client,
        Err(error) => return result(format!("error: {error}"), false),
    };

    let mut request = client
        .post(variant.url)
        .header("originator", variant.originator);
    request = if variant.url == TRANSCRIBE_URL {
        let part = reqwest::multipart::Part::bytes(wav.to_vec())
            .file_name("codex.wav")
            .mime_str("audio/wav");
        let Ok(part) = part else {
            return result("error: invalid wav content type".to_string(), false);
        };
        request
            .header("OAI-Product-Sku", "CODEX")
            .multipart(reqwest::multipart::Form::new().part("file", part))
    } else {
        request.json(&serde_json::json!({}))
    };
    if let (true, Some(session)) = (variant.authenticated, session) {
        request = request.bearer_auth(&session.access_token);
        if let Some(account_id) = session.account_id.as_deref() {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
    }

    match request.send().await {
        Ok(response) => {
            let header = |name: &str| {
                response
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("none")
                    .to_string()
            };
            let content_type = header(CONTENT_TYPE.as_str());
            let mitigated = header("cf-mitigated");
            let passed = mitigated == "none" && !content_type.starts_with("text/html");
            result(
                format!(
                    "{} {content_type} cf-mitigated={mitigated}",
                    response.status().as_u16()
                ),
                passed,
            )
        }
        Err(error) => result(format!("error: {}", error.without_url()), false),
    }
}
