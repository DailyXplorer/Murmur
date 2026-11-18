//! ChatGPT's live dictation stream. So far, only the proof that the
//! WebSocket client opens it.

#[cfg(test)]
mod proof {
    use crate::codex_auth;
    use futures_util::{SinkExt, StreamExt};
    use reqwest_websocket::{Message, Upgrade};
    use std::time::{Duration, Instant};

    /// Opens the real stream with the stored session, never refreshing it,
    /// and closes the session without sending any audio, so nothing is
    /// transcribed.
    #[test]
    #[ignore = "talks to chatgpt.com: cargo test --lib live_handshake_proof -- --ignored --nocapture"]
    fn live_handshake_proof() {
        let auth_path = codex_auth::auth_path().unwrap();
        let session = codex_auth::stored_session(&auth_path).unwrap();
        tauri::async_runtime::block_on(async {
            let opened = Instant::now();
            // An upgrade needs HTTP/1.1.
            let client = reqwest::Client::builder()
                .http1_only()
                .user_agent(crate::codex_transcribe::user_agent())
                .connect_timeout(crate::codex_transcribe::CONNECT_TIMEOUT)
                .build()
                .unwrap();
            let mut request = client
                .get("wss://chatgpt.com/backend-api/dictation/stream")
                .header("Authorization", format!("Bearer {}", session.access_token));
            if let Some(account_id) = session.account_id.as_deref() {
                request = request.header("ChatGPT-Account-Id", account_id);
            }
            let response = request
                .upgrade()
                .protocols(["chatgpt-dictation"])
                .send()
                .await
                .unwrap();
            println!(
                "status {} after {} ms",
                response.status(),
                opened.elapsed().as_millis()
            );
            let mut socket = response.into_websocket().await.unwrap();

            let start = serde_json::json!({
                "type": "session.start",
                "dictation_session_id": "6f1f6f0e-6f6b-4d5e-9c59-0d0f6a3b1c11",
                "attempt_id": "0c8f7f39-0a53-4b61-8a4f-52b0c0f6f2a7",
                "config": {
                    "input_audio_format": "pcm16",
                    "sample_rate_hz": 16000,
                    "num_channels": 1,
                    "max_buffer_size_bytes": 4194304,
                    "max_utterance_duration_ms": 30000,
                    "session_ttl_ms": 1800000,
                    "provider_mode": "streaming_sse",
                    "transcript_delivery_mode": "final_only",
                    "vad": {
                        "type": "server_vad",
                        "threshold": 0.5,
                        "prefix_padding_ms": 300,
                        "silence_duration_ms": 500,
                    },
                },
            });
            socket.send(Message::Text(start.to_string())).await.unwrap();

            let mut started = false;
            let mut closed = false;
            while let Ok(Some(Ok(Message::Text(frame)))) =
                tokio::time::timeout(Duration::from_secs(10), socket.next()).await
            {
                let event: serde_json::Value = serde_json::from_str(&frame).unwrap();
                let kind = event["type"].as_str().unwrap_or_default();
                let status = event["session"]["status"].as_str().unwrap_or_default();
                println!("{kind} {status} after {} ms", opened.elapsed().as_millis());
                if kind == "session.started" {
                    started = true;
                    let close = r#"{"type":"session.close"}"#.to_string();
                    socket.send(Message::Text(close)).await.unwrap();
                }
                if kind == "session.updated" && status == "closed" {
                    closed = true;
                    break;
                }
            }
            assert!(started, "session.started arrived");
            assert!(closed, "the session closed");
        });
    }
}
