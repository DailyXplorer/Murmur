use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
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

/// Offline, an expired token's refresh must say the network is down, or
/// a meeting gives up on its pieces instead of waiting for it.
#[tokio::test(start_paused = true)]
async fn a_refresh_that_never_connects_is_a_network_failure() {
    let error = refresh_client()
        .dns_resolver(crate::transcription_error::tests::SilentDns)
        .no_proxy()
        .build()
        .unwrap()
        .post("https://murmur.invalid/oauth/token")
        .send()
        .await
        .unwrap_err();
    assert_eq!(
        FailureKind::from_transport(&error),
        FailureKind::Network,
        "{error}"
    );
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
        tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now())).unwrap();

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
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let leftovers: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("auth.json")]);
}

#[test]
fn refresh_saves_the_tokens_when_the_caller_is_dropped_mid_request() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (endpoint, requests) = one_shot_server(
        200,
        r#"{"id_token":"new-id","access_token":"new-access","refresh_token":"rt-new"}"#,
        move || {
            arrived_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        },
    );

    // Drop the caller once the refresh token has reached the server, as a
    // cleanup timeout or a cancel would.
    tauri::async_runtime::block_on(async {
        tokio::select! {
            _ = session_for_request_at(&path, &endpoint, now()) => {
                panic!("the refresh finished before the server replied")
            }
            _ = arrived_rx => {}
        }
    });
    release_tx.send(()).unwrap();
    requests.recv_timeout(Duration::from_secs(5)).unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if saved["tokens"]["refresh_token"] == "rt-new" {
            assert_eq!(saved["tokens"]["access_token"], "new-access");
            assert_eq!(saved["tokens"]["id_token"], "new-id");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the rotated tokens were never saved"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn quitting_waits_until_a_refresh_under_way_saved_the_rotated_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (endpoint, _requests) = one_shot_server(
        200,
        r#"{"id_token":"new-id","access_token":"new-access","refresh_token":"rt-new"}"#,
        move || {
            arrived_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        },
    );
    tauri::async_runtime::block_on(async {
        tokio::select! {
            _ = session_for_request_at(&path, &endpoint, now()) => {
                panic!("the refresh finished before the server replied")
            }
            _ = arrived_rx => {}
        }
    });

    assert!(!wait_for_refreshes(Duration::from_millis(50)));
    release_tx.send(()).unwrap();
    assert!(wait_for_refreshes(Duration::from_secs(5)));

    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["tokens"]["refresh_token"], "rt-new");
}

#[test]
fn concurrent_refreshes_share_one_request() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    // The server accepts one connection only; a second refresh request
    // would fail and fall back to the old token.
    let (endpoint, requests) = one_shot_server(
        200,
        r#"{"id_token":"new-id","access_token":"new-access","refresh_token":"rt-new"}"#,
        || thread::sleep(Duration::from_millis(100)),
    );

    let (first, second) = tauri::async_runtime::block_on(async {
        tokio::join!(
            session_for_request_at(&path, &endpoint, now()),
            session_for_request_at(&path, &endpoint, now()),
        )
    });

    let expected = Session {
        access_token: "new-access".to_string(),
        account_id: Some("acct_1".to_string()),
    };
    assert_eq!(first.unwrap(), expected);
    assert_eq!(second.unwrap(), expected);
    requests.recv_timeout(Duration::from_secs(5)).unwrap();
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
        tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now())).unwrap();

    assert_eq!(session.access_token, "codex-access");
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
}

#[test]
fn refresh_saves_its_tokens_when_the_file_cannot_be_reread() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    let path_for_server = path.clone();
    let (endpoint, _requests) = one_shot_server(
        200,
        r#"{"access_token":"murmur-access","refresh_token":"murmur-rt"}"#,
        move || fs::write(&path_for_server, "{").unwrap(),
    );

    let session =
        tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now())).unwrap();

    assert_eq!(session.access_token, "murmur-access");
    let saved = file_from(fs::read_to_string(&path).unwrap());
    assert_eq!(saved.refresh_token(), Some("murmur-rt"));
}

#[test]
fn refresh_does_not_recreate_the_file_after_a_sign_out() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    let path_for_server = path.clone();
    let (endpoint, _requests) = one_shot_server(
        200,
        r#"{"access_token":"murmur-access","refresh_token":"murmur-rt"}"#,
        move || fs::remove_file(&path_for_server).unwrap(),
    );

    let error = tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now()))
        .unwrap_err();

    assert_eq!(error.kind, FailureKind::SignedOut);
    assert!(!path.exists());
}

#[test]
fn refresh_waits_for_a_slow_rewrite_and_keeps_the_new_login() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(&path, auth_json(&expiring_token(), "rt-old")).unwrap();
    let theirs = auth_json("other-account-access", "other-account-rt");
    let path_for_server = path.clone();
    let theirs_for_server = theirs.clone();
    let (endpoint, _requests) = one_shot_server(
        200,
        r#"{"access_token":"murmur-access","refresh_token":"murmur-rt"}"#,
        move || {
            // A sign-in to another account truncates the file and takes
            // longer than a few retries to write the rest.
            fs::write(&path_for_server, "").unwrap();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(400));
                fs::write(&path_for_server, theirs_for_server).unwrap();
            });
        },
    );

    let session =
        tauri::async_runtime::block_on(session_for_request_at(&path, &endpoint, now())).unwrap();

    assert_eq!(session.access_token, "other-account-access");
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
fn stored_session_never_refreshes_or_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    let soon = jwt(serde_json::json!({"exp": Utc::now().timestamp() + 60}));
    let original = auth_json(&soon, "rt");
    fs::write(&path, &original).unwrap();

    assert_eq!(stored_session(&path).unwrap().access_token, soon);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);

    let expired = jwt(serde_json::json!({"exp": Utc::now().timestamp() - 1}));
    fs::write(&path, auth_json(&expired, "rt")).unwrap();
    assert_eq!(
        stored_session(&path).unwrap_err().kind,
        FailureKind::SessionExpired
    );
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
    // A half-written file is not a sign-out: it must not end a meeting run.
    assert_eq!(error.kind, FailureKind::Other);

    fs::remove_file(&path).unwrap();
    assert_eq!(
        AuthFile::read(&path).unwrap_err().kind,
        FailureKind::SignedOut
    );
}
