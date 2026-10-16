//! Apple's on-device model, through the Swift bridge in `swift/`. The bridge
//! answers through a callback from a background task, so nothing here blocks
//! a thread while the model runs; the caller's deadline bounds the wait.

use serde::Serialize;
use specta::Type;
use std::ffi::{c_char, c_void, CStr, CString};
use tokio::sync::oneshot;

/// The on-device model's context window, in tokens, shared by the
/// instructions, the prompt and the reply.
const CONTEXT_TOKENS: usize = 4_096;
/// Room for the session's own framing around instructions and prompt.
const FRAMING_TOKENS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AppleIntelligenceStatus {
    Available,
    DeviceNotEligible,
    AppleIntelligenceNotEnabled,
    ModelNotReady,
    UnsupportedOs,
    Unavailable,
}

impl AppleIntelligenceStatus {
    fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Available,
            1 => Self::DeviceNotEligible,
            2 => Self::AppleIntelligenceNotEnabled,
            3 => Self::ModelNotReady,
            4 => Self::UnsupportedOs,
            _ => Self::Unavailable,
        }
    }
}

/// Why the on-device model gave no reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppleError {
    /// Apple's safety guardrails flagged the dictation or the reply.
    Guardrail,
    /// The dictation does not fit the context window.
    ContextTooLong,
    Timeout,
    Refused,
    Unavailable,
    Other,
}

impl AppleError {
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Guardrail,
            2 => Self::ContextTooLong,
            3 => Self::Timeout,
            4 => Self::Refused,
            5 => Self::Unavailable,
            _ => Self::Other,
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod bridge {
    use std::ffi::{c_char, c_void};

    pub type Callback = extern "C" fn(*mut c_void, i32, *const c_char);

    extern "C" {
        pub fn murmur_apple_availability() -> i32;
        pub fn murmur_apple_cleanup(
            instructions: *const c_char,
            prompt: *const c_char,
            max_response_tokens: i32,
            context: *mut c_void,
            callback: Callback,
        );
    }
}

pub fn status() -> AppleIntelligenceStatus {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: takes no arguments and only reads the model's availability.
        AppleIntelligenceStatus::from_code(unsafe { bridge::murmur_apple_availability() })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        AppleIntelligenceStatus::DeviceNotEligible
    }
}

/// A deliberately high estimate: a token is three or more characters of
/// ASCII text, and at most one character of any other script.
fn estimated_tokens(text: &str) -> usize {
    let ascii = text.chars().filter(char::is_ascii).count();
    ascii.div_ceil(3) + text.chars().count() - ascii
}

/// The reply's token cap, or `None` when instructions, prompt and the
/// longest acceptable reply (see `reply::max_output_chars`) would not fit
/// the context window. A reply that reaches the cap fails the reply guard
/// anyway, so the cap only stops a runaway early.
pub fn response_token_budget(instructions: &str, prompt: &str, dictation: &str) -> Option<usize> {
    let input = estimated_tokens(instructions) + estimated_tokens(prompt) + FRAMING_TOKENS;
    let longest_reply = estimated_tokens(dictation) * 3 / 2 + 16;
    (input + longest_reply <= CONTEXT_TOKENS).then_some(longest_reply)
}

type Reply = Result<String, AppleError>;

/// Called exactly once by the bridge, on its own thread.
extern "C" fn deliver(context: *mut c_void, status: i32, text: *const c_char) {
    // SAFETY: `context` is the `Box` leaked in `complete`, and the bridge
    // calls back once per request, so it is reclaimed exactly once.
    let sender = unsafe { Box::from_raw(context.cast::<oneshot::Sender<Reply>>()) };
    let reply = if status == 0 && !text.is_null() {
        // SAFETY: the bridge passes a NUL-terminated string that lives for
        // the duration of this call; it is copied before returning.
        Ok(unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned())
    } else {
        Err(AppleError::from_code(status))
    };
    // The caller may have given up at its deadline; the reply is dropped then.
    let _ = sender.send(reply);
}

/// One request to a fresh on-device session.
pub async fn complete(
    instructions: &str,
    prompt: &str,
    max_response_tokens: usize,
) -> Result<String, AppleError> {
    let (Ok(instructions), Ok(prompt)) = (CString::new(instructions), CString::new(prompt)) else {
        return Err(AppleError::Other);
    };
    let max_response_tokens = i32::try_from(max_response_tokens).unwrap_or(i32::MAX);
    let (sender, receiver) = oneshot::channel::<Reply>();
    let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
    #[cfg(target_arch = "aarch64")]
    // SAFETY: the strings outlive the call, which copies them before
    // returning; `context` is reclaimed by `deliver`.
    unsafe {
        bridge::murmur_apple_cleanup(
            instructions.as_ptr(),
            prompt.as_ptr(),
            max_response_tokens,
            context,
            deliver,
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (instructions, prompt, max_response_tokens);
        deliver(context, 5, std::ptr::null());
    }
    receiver.await.unwrap_or(Err(AppleError::Other))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::prompt;

    fn budget_for(dictation: &str) -> Option<usize> {
        response_token_budget(
            prompt::INSTRUCTIONS,
            &prompt::user_message(dictation, &prompt::PromptContext::default()),
            dictation,
        )
    }

    #[test]
    fn token_estimate_overcounts_rather_than_under() {
        assert_eq!(estimated_tokens("hello"), 2);
        assert_eq!(estimated_tokens("café"), 2);
        assert_eq!(estimated_tokens("我们明天开会"), 6);
    }

    #[test]
    fn a_long_dictation_is_not_sent_to_the_small_model() {
        let sentence = "we should move the launch to next week ";
        assert_eq!(budget_for(sentence), Some(35));
        assert!(budget_for(&sentence.repeat(60)).is_some());
        assert_eq!(budget_for(&sentence.repeat(120)), None);
        assert!(budget_for(&"我们明天三点开会然后讨论预算".repeat(50)).is_some());
        assert_eq!(
            budget_for(&"我们明天三点开会然后讨论预算".repeat(120)),
            None
        );
    }

    /// A strong link to either would stop the app from launching on macOS
    /// before 26, or before 12.
    #[test]
    fn the_bridge_is_weak_linked_to_what_older_macos_lacks() {
        let executable = std::env::current_exe().unwrap();
        let output = std::process::Command::new("otool")
            .arg("-l")
            .arg(&executable)
            .output()
            .unwrap();
        let load_commands = String::from_utf8_lossy(&output.stdout);
        let linked = |kind: &str, library: &str| {
            load_commands
                .split("Load command")
                .any(|block| block.contains(&format!("cmd {kind}\n")) && block.contains(library))
        };
        assert!(linked("LC_LOAD_WEAK_DYLIB", "FoundationModels.framework"));
        assert!(!linked("LC_LOAD_DYLIB", "FoundationModels.framework"));
        assert!(!linked("LC_LOAD_DYLIB", "libswift_Concurrency.dylib"));
    }

    #[test]
    fn bridge_codes_map_to_states_and_errors() {
        assert_eq!(
            AppleIntelligenceStatus::from_code(2),
            AppleIntelligenceStatus::AppleIntelligenceNotEnabled
        );
        assert_eq!(
            AppleIntelligenceStatus::from_code(99),
            AppleIntelligenceStatus::Unavailable
        );
        assert_eq!(AppleError::from_code(1), AppleError::Guardrail);
        assert_eq!(AppleError::from_code(2), AppleError::ContextTooLong);
        assert_eq!(AppleError::from_code(99), AppleError::Other);
    }

    #[test]
    fn a_status_serializes_as_the_frontend_reads_it() {
        assert_eq!(
            serde_json::to_value(AppleIntelligenceStatus::AppleIntelligenceNotEnabled).unwrap(),
            "appleIntelligenceNotEnabled"
        );
        assert_eq!(
            serde_json::to_value(AppleIntelligenceStatus::UnsupportedOs).unwrap(),
            "unsupportedOs"
        );
    }
}
