use crate::codex_login::{self, LoginConfig, SignInError, SignInSlot};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

static SIGN_IN: SignInSlot = SignInSlot::new();

#[tauri::command]
#[specta::specta]
/// Signs in to ChatGPT in the browser, as Codex does, and saves the session
/// Codex and Murmur share. A sign-in already under way is cancelled first.
pub async fn sign_in_with_chatgpt(app: AppHandle) -> Result<(), SignInError> {
    let auth_path = crate::codex_auth::auth_path().map_err(|_| SignInError::SaveFailed)?;
    let config = LoginConfig::new(auth_path);
    SIGN_IN
        .run(|cancelled| {
            codex_login::sign_in(
                &config,
                |url| {
                    app.opener()
                        .open_url(url, None::<&str>)
                        .map_err(|error| error.to_string())
                },
                async move {
                    let _ = cancelled.await;
                },
            )
        })
        .await
}

#[tauri::command]
#[specta::specta]
pub fn cancel_chatgpt_sign_in() {
    SIGN_IN.cancel();
}

#[tauri::command]
#[specta::specta]
/// Signs out as `codex logout` does, so Codex is signed out too.
pub async fn sign_out_of_chatgpt() -> Result<(), String> {
    let auth_path = crate::codex_auth::auth_path().map_err(|error| error.detail)?;
    codex_login::sign_out(&LoginConfig::new(auth_path)).await
}
