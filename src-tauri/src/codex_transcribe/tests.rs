use super::*;
use crate::{OperationId, ProcessingOperation};
use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn parse_json_text_field() {
    assert_eq!(
        parse_transcript(r#"{"text":"Bonjour"}"#).unwrap(),
        "Bonjour"
    );
}

#[test]
fn parse_json_transcript_field() {
    assert_eq!(
        parse_transcript(r#"{"transcript":"Hello"}"#).unwrap(),
        "Hello"
    );
}

#[test]
fn parse_plain_text_response() {
    assert_eq!(
        parse_transcript("  Bonjour le monde  ").unwrap(),
        "Bonjour le monde"
    );
}

#[test]
fn reject_json_without_transcript() {
    assert!(parse_transcript(r#"{"status":"ok"}"#).is_err());
}

#[test]
fn normalizes_regional_language_for_request() {
    assert_eq!(api_language("fr-FR"), "fr");
    assert_eq!(api_language("zh-Hant"), "zh");
}

#[test]
fn encode_wav_has_header_and_samples() {
    let wav = pcm_f32_to_wav_bytes(&[0.5; 160]).unwrap();
    assert_eq!(wav.len(), upload_size_for_samples(160));
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
}

#[test]
fn timeout_scales_with_upload_size() {
    assert_eq!(request_timeout(100_000, 1), Duration::from_secs(61));
    let fifteen_minutes = upload_size_for_samples(15 * 60 * SAMPLE_RATE as usize);
    assert_eq!(
        request_timeout(fifteen_minutes, 1),
        Duration::from_secs(499)
    );
}

#[test]
fn parallel_uploads_each_get_their_share_of_the_uplink() {
    let five_minutes = upload_size_for_samples(5 * 60 * SAMPLE_RATE as usize);
    let alone = request_timeout(five_minutes, 1);
    let shared = request_timeout(five_minutes, 3);
    assert_eq!(alone, Duration::from_secs(206));
    // Three at once on the same uplink take three times as long to send.
    assert_eq!(shared, Duration::from_secs(499));
    assert_eq!(request_timeout(five_minutes, 0), alone);
}

/// A dictation sent while meeting pieces upload shares the uplink with
/// them, so its timeout counts them.
#[test]
fn an_upload_counts_the_uploads_already_in_flight() {
    let meeting_pieces: Vec<Upload> = (0..3).map(|_| Upload::begin()).collect();
    let dictation = Upload::begin();
    // Other tests may be uploading too.
    assert!(dictation.sharing >= 4, "{}", dictation.sharing);
    drop(meeting_pieces);
}

#[test]
fn soft_limit_is_reached_after_about_thirteen_minutes() {
    let samples_per_minute = 60 * SAMPLE_RATE as usize;
    assert!(upload_size_for_samples(13 * samples_per_minute) < UPLOAD_SOFT_LIMIT_BYTES);
    assert!(upload_size_for_samples(14 * samples_per_minute) > UPLOAD_SOFT_LIMIT_BYTES);
}

/// `audio` as a Codex request with no language.
fn upload(audio: &[u8]) -> UploadRequest<'_> {
    UploadRequest {
        identity: RequestIdentity::Codex,
        format: AudioFormat::Wav,
        audio,
        language: None,
        timeout: request_timeout(audio.len(), 1),
    }
}

fn reply_once(status_line: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        use std::io::Write;
        let (mut peer, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 8192];
        let _ = peer.read(&mut buffer);
        let body = r#"{"detail":"nope"}"#;
        let _ = write!(
            peer,
            "HTTP/1.1 {status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    endpoint
}

fn failure_kind_for(status_line: &'static str) -> FailureKind {
    let endpoint = reply_once(status_line);
    let session = Session {
        access_token: "test-token".to_string(),
        account_id: None,
    };
    let operation = ProcessingOperation::new(OperationId(3));
    match tauri::async_runtime::block_on(transcribe_with_session_at(
        &endpoint,
        &session,
        &upload(b"wav"),
        &operation,
    )) {
        Err(RequestError::Failed(error)) => error.kind,
        _ => panic!("{status_line} must fail"),
    }
}

#[test]
fn http_failures_are_classified() {
    assert_eq!(
        failure_kind_for("401 Unauthorized"),
        FailureKind::SessionExpired
    );
    assert_eq!(
        failure_kind_for("429 Too Many Requests"),
        FailureKind::RateLimited
    );
    assert_eq!(
        failure_kind_for("502 Bad Gateway"),
        FailureKind::ServerError
    );
    assert_eq!(
        failure_kind_for("413 Payload Too Large"),
        FailureKind::TooLarge
    );
}

const CODEX_QUOTA_SPENT: (&str, &str) = (
    "429 Too Many Requests",
    r#"{"detail":"Codex dictation usage limit reached"}"#,
);
const CHATGPT_QUOTA_SPENT: (&str, &str) = (
    "429 Too Many Requests",
    r#"{"detail":"Dictation usage limit reached"}"#,
);
const TRANSCRIBED: (&str, &str) = ("200 OK", r#"{"text":"Hello"}"#);

type Replies = &'static [(&'static str, &'static str)];

/// Answers one connection per reply, in order, and sends back each
/// request's head, lowercased.
fn serve(replies: Replies) -> (String, mpsc::Receiver<String>) {
    serve_and_send(replies, |(head, _)| head)
}

/// `serve`, sending back each request's body instead.
fn serve_bodies(replies: Replies) -> (String, mpsc::Receiver<Vec<u8>>) {
    serve_and_send(replies, |(_, body)| body)
}

fn serve_and_send<T: Send + 'static>(
    replies: Replies,
    part: fn((String, Vec<u8>)) -> T,
) -> (String, mpsc::Receiver<T>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (requests_tx, requests_rx) = mpsc::channel();
    thread::spawn(move || {
        use std::io::Write;
        for (status_line, body) in replies {
            let (mut peer, _) = listener.accept().unwrap();
            requests_tx.send(part(read_request(&mut peer))).unwrap();
            let _ = write!(
                peer,
                "HTTP/1.1 {status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (endpoint, requests_rx)
}

/// A request's head, lowercased, and its body.
fn read_request(peer: &mut std::net::TcpStream) -> (String, Vec<u8>) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = peer.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
        let Some(end) = text.find("\r\n\r\n") else {
            assert!(read > 0, "request ended before its head");
            continue;
        };
        let head = &text[..end];
        let body_length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|length| length.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if read == 0 || request.len() >= end + 4 + body_length {
            return (head.to_string(), request[end + 4..].to_vec());
        }
    }
}

fn signed_in(directory: &Path) -> std::path::PathBuf {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD
            .encode(serde_json::json!({"exp": chrono::Utc::now().timestamp() + 3600}).to_string())
    );
    let path = directory.join("auth.json");
    std::fs::write(
        &path,
        serde_json::json!({"tokens": {"access_token": token, "refresh_token": "rt"}}).to_string(),
    )
    .unwrap();
    path
}

fn quota() -> DictationQuota {
    DictationQuota {
        codex_spent_at: Mutex::new(None),
    }
}

fn codex_quota_spent_at(at: SystemTime) -> DictationQuota {
    let quota = quota();
    assert!(quota.codex_spent(at));
    quota
}

fn upload_format() -> UploadFormat {
    UploadFormat {
        flac_refused_at: Mutex::new(None),
    }
}

fn dictate(endpoint: &str, auth_path: &Path, quota: &DictationQuota) -> Result<String> {
    dictate_as(endpoint, auth_path, quota, &upload_format())
}

fn dictate_as(
    endpoint: &str,
    auth_path: &Path,
    quota: &DictationQuota,
    upload_format: &UploadFormat,
) -> Result<String> {
    let operation = ProcessingOperation::new(OperationId(9));
    tauri::async_runtime::block_on(transcribe_with(
        endpoint,
        Some(auth_path),
        &[0.1; 160],
        None,
        1,
        &operation,
        Learned {
            quota,
            upload_format,
        },
    ))
}

fn sent_as_codex(head: &str) -> bool {
    head.contains("\r\noriginator: codex desktop") && head.contains("\r\noai-product-sku: codex")
}

fn sent_as_chatgpt(head: &str) -> bool {
    head.contains("\r\nauthorization: bearer ")
        && !head.contains("\r\noriginator:")
        && !head.contains("\r\noai-product-sku:")
}

#[test]
fn spent_codex_dictation_quota_resends_on_the_chatgpt_quota() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, heads) = serve(&[CODEX_QUOTA_SPENT, TRANSCRIBED]);
    let quota = quota();

    assert_eq!(dictate(&endpoint, &auth_path, &quota).unwrap(), "Hello");
    let heads: Vec<String> = heads.try_iter().collect();
    assert_eq!(heads.len(), 2);
    assert!(sent_as_codex(&heads[0]), "first try as Codex: {}", heads[0]);
    assert!(
        sent_as_chatgpt(&heads[1]),
        "resent as ChatGPT: {}",
        heads[1]
    );

    let (endpoint, heads) = serve(&[TRANSCRIBED]);
    assert_eq!(dictate(&endpoint, &auth_path, &quota).unwrap(), "Hello");
    let head = heads.recv().unwrap();
    assert!(sent_as_chatgpt(&head), "next dictation skips Codex: {head}");
}

#[test]
fn other_rate_limits_keep_the_codex_identity() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, heads) = serve(&[("429 Too Many Requests", r#"{"detail":"nope"}"#)]);

    let error = dictate(&endpoint, &auth_path, &quota()).unwrap_err();
    assert_eq!(
        TranscriptionError::kind_of(&error),
        FailureKind::RateLimited
    );
    let heads: Vec<String> = heads.try_iter().collect();
    assert_eq!(heads.len(), 1);
    assert!(sent_as_codex(&heads[0]), "{}", heads[0]);
}

#[test]
fn a_remembered_codex_quota_goes_straight_to_chatgpt_until_it_ends() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());

    let (endpoint, heads) = serve(&[TRANSCRIBED]);
    let spent = codex_quota_spent_at(SystemTime::now());
    assert_eq!(dictate(&endpoint, &auth_path, &spent).unwrap(), "Hello");
    let head = heads.recv().unwrap();
    assert!(sent_as_chatgpt(&head), "{head}");

    let (endpoint, heads) = serve(&[TRANSCRIBED]);
    let ended = codex_quota_spent_at(SystemTime::now() - CODEX_QUOTA_BACKOFF);
    assert_eq!(dictate(&endpoint, &auth_path, &ended).unwrap(), "Hello");
    let head = heads.recv().unwrap();
    assert!(sent_as_codex(&head), "{head}");
}

#[test]
fn the_codex_quota_is_skipped_for_one_backoff() {
    let quota = quota();
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    assert!(quota.codex_spent(start));
    // A parallel upload hitting the same quota does not restart the window.
    assert!(!quota.codex_spent(start + Duration::from_secs(1)));
    // Windows clocks count in 100 ns steps: a nanosecond earlier is
    // the same instant there.
    let last = start + CODEX_QUOTA_BACKOFF - Duration::from_micros(1);
    assert_eq!(quota.identity(last), RequestIdentity::ChatGpt);
    assert_eq!(
        quota.identity(start + CODEX_QUOTA_BACKOFF),
        RequestIdentity::Codex
    );
}

#[test]
fn a_clock_set_back_ends_the_codex_quota_window() {
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let quota = codex_quota_spent_at(start);
    let set_back = start - Duration::from_secs(1);
    assert_eq!(quota.identity(set_back), RequestIdentity::Codex);
    assert!(quota.codex_spent(set_back), "a new window opens");
    assert_eq!(quota.identity(set_back), RequestIdentity::ChatGpt);
}

#[test]
fn only_a_dictation_usage_limit_429_is_a_spent_quota() {
    let codex = CODEX_QUOTA_SPENT.1;
    assert!(is_dictation_quota_spent(429, codex));
    assert!(is_dictation_quota_spent(
        429,
        r#"{"detail":"DICTATION USAGE LIMIT reached"}"#
    ));
    assert!(!is_dictation_quota_spent(403, codex));
    assert!(!is_dictation_quota_spent(429, r#"{"detail":"nope"}"#));
    assert!(!is_dictation_quota_spent(429, "dictation usage limit"));
}

#[test]
fn both_dictation_quotas_spent_is_rate_limited() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, heads) = serve(&[CODEX_QUOTA_SPENT, CHATGPT_QUOTA_SPENT]);

    let error = dictate(&endpoint, &auth_path, &quota()).unwrap_err();
    assert_eq!(
        TranscriptionError::kind_of(&error),
        FailureKind::RateLimited
    );
    assert!(error.to_string().contains("Dictation usage limit reached"));
    let heads: Vec<String> = heads.try_iter().collect();
    assert_eq!(heads.len(), 2);
    assert!(sent_as_chatgpt(&heads[1]), "{}", heads[1]);
}

#[test]
fn a_spent_chatgpt_quota_goes_back_to_codex() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, heads) = serve(&[CHATGPT_QUOTA_SPENT, TRANSCRIBED]);
    let quota = codex_quota_spent_at(SystemTime::now());

    assert_eq!(dictate(&endpoint, &auth_path, &quota).unwrap(), "Hello");
    let heads: Vec<String> = heads.try_iter().collect();
    assert_eq!(heads.len(), 2);
    assert!(sent_as_chatgpt(&heads[0]), "{}", heads[0]);
    assert!(sent_as_codex(&heads[1]), "resent as Codex: {}", heads[1]);

    let (endpoint, heads) = serve(&[TRANSCRIBED]);
    assert_eq!(dictate(&endpoint, &auth_path, &quota).unwrap(), "Hello");
    let head = heads.recv().unwrap();
    assert!(sent_as_codex(&head), "the window is cleared: {head}");
}

#[test]
fn both_quotas_spent_from_chatgpt_reopens_the_window() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, heads) = serve(&[CHATGPT_QUOTA_SPENT, CODEX_QUOTA_SPENT]);
    let quota = codex_quota_spent_at(SystemTime::now());

    let error = dictate(&endpoint, &auth_path, &quota).unwrap_err();
    assert_eq!(
        TranscriptionError::kind_of(&error),
        FailureKind::RateLimited
    );
    let heads: Vec<String> = heads.try_iter().collect();
    assert_eq!(heads.len(), 2);
    assert!(sent_as_codex(&heads[1]), "{}", heads[1]);

    let (endpoint, heads) = serve(&[TRANSCRIBED]);
    assert_eq!(dictate(&endpoint, &auth_path, &quota).unwrap(), "Hello");
    let head = heads.recv().unwrap();
    assert!(sent_as_chatgpt(&head), "the window is open again: {head}");
}

#[test]
fn unreachable_service_is_a_network_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let session = Session {
        access_token: "test-token".to_string(),
        account_id: None,
    };
    let operation = ProcessingOperation::new(OperationId(4));
    match tauri::async_runtime::block_on(transcribe_with_session_at(
        &endpoint,
        &session,
        &upload(b"wav"),
        &operation,
    )) {
        Err(RequestError::Failed(error)) => assert_eq!(error.kind, FailureKind::Network),
        _ => panic!("a closed port must fail"),
    }
}

#[test]
fn cancellation_closes_a_stalled_codex_http_peer() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buffer = [0_u8; 4096];
        let _ = peer.read(&mut buffer);
        accepted_tx.send(()).unwrap();
        loop {
            match peer.read(&mut buffer) {
                Ok(0) => {
                    closed_tx.send(true).unwrap();
                    return;
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    closed_tx.send(false).unwrap();
                    return;
                }
                Err(error) => panic!("stalled peer read failed: {error}"),
            }
        }
    });

    let operation = ProcessingOperation::new(OperationId(7));
    let session = Session {
        access_token: "test-token".to_string(),
        account_id: None,
    };
    let request_operation = operation.clone();
    let request = tauri::async_runtime::spawn(async move {
        transcribe_with_session_at(
            &endpoint,
            &session,
            &upload(b"test wav"),
            &request_operation,
        )
        .await
    });

    accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(operation.cancel());
    let result = tauri::async_runtime::block_on(request).unwrap();
    assert!(matches!(result, Err(RequestError::Cancelled)));
    assert!(closed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    server.join().unwrap();
}

#[test]
fn cancellation_stops_waiting_for_a_token_refresh() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    // Expires within the refresh window: the request refreshes first.
    let expiring = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD
            .encode(serde_json::json!({"exp": chrono::Utc::now().timestamp() + 60}).to_string())
    );
    let auth = |access_token: &str| {
        serde_json::json!({"tokens": {"access_token": access_token, "refresh_token": "rt"}})
            .to_string()
    };
    std::fs::write(&path, auth(&expiring)).unwrap();
    let operation = ProcessingOperation::new(OperationId(8));
    let request_operation = operation.clone();
    let request_path = path.clone();

    tauri::async_runtime::block_on(async {
        // Another refresh is under way.
        let other_refresh = codex_auth::REFRESH_LOCK.lock().await;
        let request = tauri::async_runtime::spawn(async move {
            transcribe_at(
                "http://127.0.0.1:9",
                Some(&request_path),
                &[0.1; 160],
                None,
                1,
                &request_operation,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(operation.cancel());
        let result = tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .expect("a cancel must not wait for the refresh")
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        // The other refresh saved new tokens, so the queued one ends offline.
        std::fs::write(&path, auth("fresh")).unwrap();
        drop(other_refresh);
    });
    assert!(codex_auth::wait_for_refreshes(Duration::from_secs(5)));
}

#[test]
#[ignore]
fn live_codex_session_transcribes() {
    let sample_rate = SAMPLE_RATE as f32;
    let samples: Vec<f32> = (0..SAMPLE_RATE)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.2
        })
        .collect();
    let operation = ProcessingOperation::new(OperationId(1));
    let result = tauri::async_runtime::block_on(transcribe(&samples, Some("fr"), &operation));
    assert!(
        result.is_ok(),
        "Codex transcription failed: {:?}",
        result.err()
    );
}

fn has(body: &[u8], part: &[u8]) -> bool {
    body.windows(part.len()).any(|window| window == part)
}

/// A tone that swells past full scale, so some samples clip.
fn swelling_tone(samples: usize) -> Vec<f32> {
    (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            (2.0 * std::f32::consts::PI * 220.0 * t).sin() * (0.1 + 1.2 * t)
        })
        .collect()
}

#[test]
fn flac_holds_the_samples_wav_holds_in_fewer_bytes() {
    let samples = swelling_tone(SAMPLE_RATE as usize);

    let flac = pcm_f32_to_flac_bytes(&samples).unwrap();
    let wav = pcm_f32_to_wav_bytes(&samples).unwrap();

    assert_eq!(uploaded::pcm(&flac), uploaded::pcm(&wav));
    assert!(
        flac.len() < wav.len(),
        "{} against {}",
        flac.len(),
        wav.len()
    );
}

#[test]
fn flac_loses_no_sample_at_the_edge_of_a_block() {
    let block = flacenc::config::Encoder::default().block_size;
    for length in [1, block - 1, block, block + 1] {
        let samples = swelling_tone(length);
        let flac = pcm_f32_to_flac_bytes(&samples).unwrap();
        let wav = pcm_f32_to_wav_bytes(&samples).unwrap();
        assert_eq!(
            uploaded::pcm(&flac),
            uploaded::pcm(&wav),
            "{length} samples"
        );
    }
}

const REFUSED: (&str, &str) = ("400 Bad Request", r#"{"detail":"Unsupported file"}"#);

#[test]
fn a_dictation_is_uploaded_as_flac() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, bodies) = serve_bodies(&[TRANSCRIBED]);

    assert_eq!(dictate(&endpoint, &auth_path, &quota()).unwrap(), "Hello");

    let body = bodies.recv().unwrap();
    assert!(has(&body, b"filename=\"codex.flac\""));
    assert!(has(&body, b"audio/flac"));
    assert!(has(&body, b"fLaC"));
}

#[test]
fn refused_flac_goes_again_as_wav_and_later_uploads_start_as_wav() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, bodies) = serve_bodies(&[REFUSED, TRANSCRIBED, TRANSCRIBED]);
    let (quota, upload_format) = (quota(), upload_format());

    let first = dictate_as(&endpoint, &auth_path, &quota, &upload_format);
    assert_eq!(first.unwrap(), "Hello");
    let refused = bodies.recv().unwrap();
    assert!(has(&refused, b"filename=\"codex.flac\""));
    let resent = bodies.recv().unwrap();
    assert!(has(&resent, b"filename=\"codex.wav\""));
    assert!(has(&resent, b"audio/wav"));
    assert!(has(&resent, b"RIFF"));

    let second = dictate_as(&endpoint, &auth_path, &quota, &upload_format);
    assert_eq!(second.unwrap(), "Hello");
    let next = bodies.recv().unwrap();
    assert!(
        has(&next, b"filename=\"codex.wav\""),
        "FLAC is not tried again"
    );
}

#[test]
fn audio_refused_in_both_forms_fails_and_flac_stays_the_first_choice() {
    let directory = tempfile::tempdir().unwrap();
    let auth_path = signed_in(directory.path());
    let (endpoint, bodies) = serve_bodies(&[REFUSED, REFUSED, TRANSCRIBED]);
    let (quota, upload_format) = (quota(), upload_format());

    let error = dictate_as(&endpoint, &auth_path, &quota, &upload_format).unwrap_err();
    assert_eq!(TranscriptionError::kind_of(&error), FailureKind::Rejected);
    assert!(has(&bodies.recv().unwrap(), b"filename=\"codex.flac\""));
    assert!(has(&bodies.recv().unwrap(), b"filename=\"codex.wav\""));

    let next = dictate_as(&endpoint, &auth_path, &quota, &upload_format);
    assert_eq!(next.unwrap(), "Hello");
    assert!(has(&bodies.recv().unwrap(), b"filename=\"codex.flac\""));
}

#[test]
fn flac_is_tried_again_an_hour_after_a_refusal() {
    let refused = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let upload_format = upload_format();
    assert_eq!(upload_format.preferred(refused), AudioFormat::Flac);

    assert!(upload_format.flac_refused(refused));
    assert!(
        !upload_format.flac_refused(refused + Duration::from_secs(1)),
        "parallel uploads report the refusal once"
    );

    let almost = FLAC_RETRY_AFTER - Duration::from_secs(1);
    assert_eq!(upload_format.preferred(refused + almost), AudioFormat::Wav);
    assert_eq!(
        upload_format.preferred(refused + FLAC_RETRY_AFTER),
        AudioFormat::Flac
    );
    assert_eq!(
        upload_format.preferred(refused - Duration::from_secs(1)),
        AudioFormat::Flac,
        "a clock set back ends the window"
    );
}

/// Sends the 16 kHz mono WAV of speech that `MURMUR_LIVE_CLIP` names with
/// the stored session, never refreshing it, as FLAC and as WAV, and prints
/// both transcripts for a person to compare.
#[test]
#[ignore]
fn live_codex_session_takes_flac() {
    let clip = std::env::var("MURMUR_LIVE_CLIP").expect("MURMUR_LIVE_CLIP names a WAV file");
    let samples: Vec<f32> = hound::WavReader::open(clip)
        .unwrap()
        .samples::<i16>()
        .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
        .collect();
    let auth_path = codex_auth::auth_path().unwrap();
    let session = codex_auth::stored_session(&auth_path).unwrap();
    let operation = ProcessingOperation::new(OperationId(2));

    for format in [AudioFormat::Flac, AudioFormat::Wav] {
        let audio = format.encode(&samples).unwrap();
        let request = UploadRequest {
            format,
            ..upload(&audio)
        };
        let started = std::time::Instant::now();
        let text = tauri::async_runtime::block_on(transcribe_with_session_at(
            TRANSCRIBE_URL,
            &session,
            &request,
            &operation,
        ));
        let Ok(text) = text else {
            panic!("Codex refused the clip as {format:?}");
        };
        println!(
            "{format:?}, {} bytes, {:?}: {text}",
            audio.len(),
            started.elapsed()
        );
    }
}
