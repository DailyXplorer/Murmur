//! Which engine transcribes a dictation: Parakeet on this Mac or Codex in the
//! cloud. `route` decides before any audio is sent, and `reroute` decides once
//! after the chosen engine failed. Both are pure, so the whole table is tested.

pub mod parakeet_languages;

use crate::transcription_error::FailureKind;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// Parakeet TDT 0.6B v3 on this Mac.
    Local,
    /// Codex through the user's ChatGPT session.
    #[default]
    Cloud,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cloud => "cloud",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "cloud" => Some(Self::Cloud),
            _ => None,
        }
    }
}

/// Why a dictation was transcribed by the engine the user did not choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    /// Local chosen: the model is not downloaded or not ready yet.
    ModelNotReady,
    /// Local chosen: the local run failed.
    LocalFailed,
    /// Local chosen: the dictation language is outside Parakeet's 25.
    LanguageNotSupported,
    /// Cloud chosen: ChatGPT refused for the account's usage limit.
    CloudLimitReached,
    /// Cloud chosen: ChatGPT failed for any other reason.
    CloudFailed,
}

impl FallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelNotReady => "model_not_ready",
            Self::LocalFailed => "local_failed",
            Self::LanguageNotSupported => "language_not_supported",
            Self::CloudLimitReached => "cloud_limit_reached",
            Self::CloudFailed => "cloud_failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::ModelNotReady,
            Self::LocalFailed,
            Self::LanguageNotSupported,
            Self::CloudLimitReached,
            Self::CloudFailed,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == value)
    }
}

/// The engine that transcribes a dictation, and why it is not the chosen one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineUsed {
    pub engine: Engine,
    pub fallback: Option<FallbackReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Run(EngineUsed),
    /// The local model cannot serve and no ChatGPT account can take over.
    Refuse(FallbackReason),
}

/// What the dictation settings and this Mac allow at the moment of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conditions<'a> {
    pub model_ready: bool,
    pub signed_in: bool,
    /// The dictation language setting, "auto" included.
    pub language: &'a str,
}

/// Where a dictation goes before any audio is sent. In Cloud mode nothing
/// changes from a Codex-only Murmur. The language is judged from the setting
/// only: Parakeet has no reliable signal that it does not know a language.
pub fn route(choice: Engine, conditions: Conditions) -> Route {
    match choice {
        Engine::Cloud => Route::Run(EngineUsed {
            engine: Engine::Cloud,
            fallback: None,
        }),
        Engine::Local => {
            if !parakeet_languages::supports(conditions.language) {
                to_cloud(FallbackReason::LanguageNotSupported, conditions)
            } else if !conditions.model_ready {
                to_cloud(FallbackReason::ModelNotReady, conditions)
            } else {
                Route::Run(EngineUsed {
                    engine: Engine::Local,
                    fallback: None,
                })
            }
        }
    }
}

/// Where a dictation goes after `failed` returned an error of `kind`, or
/// `None` when that error stands. A fallback is never re-routed, so a
/// dictation tries at most two engines. The caller handles cancellation
/// before asking.
pub fn reroute(failed: EngineUsed, kind: FailureKind, conditions: Conditions) -> Option<Route> {
    if failed.fallback.is_some() {
        return None;
    }
    match failed.engine {
        Engine::Local => Some(to_cloud(FallbackReason::LocalFailed, conditions)),
        Engine::Cloud => {
            let local_can_serve =
                conditions.model_ready && parakeet_languages::supports(conditions.language);
            local_can_serve.then(|| {
                Route::Run(EngineUsed {
                    engine: Engine::Local,
                    fallback: Some(if kind == FailureKind::RateLimited {
                        FallbackReason::CloudLimitReached
                    } else {
                        FallbackReason::CloudFailed
                    }),
                })
            })
        }
    }
}

fn to_cloud(reason: FallbackReason, conditions: Conditions) -> Route {
    if conditions.signed_in {
        Route::Run(EngineUsed {
            engine: Engine::Cloud,
            fallback: Some(reason),
        })
    } else {
        Route::Refuse(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FallbackReason::*;

    const READY: bool = true;
    const NOT_READY: bool = false;
    const ACCOUNT: bool = true;
    const NO_ACCOUNT: bool = false;

    fn when(model_ready: bool, signed_in: bool, language: &str) -> Conditions<'_> {
        Conditions {
            model_ready,
            signed_in,
            language,
        }
    }

    fn local(fallback: Option<FallbackReason>) -> Route {
        Route::Run(EngineUsed {
            engine: Engine::Local,
            fallback,
        })
    }

    fn cloud(fallback: Option<FallbackReason>) -> Route {
        Route::Run(EngineUsed {
            engine: Engine::Cloud,
            fallback,
        })
    }

    fn used(route: Route) -> EngineUsed {
        match route {
            Route::Run(used) => used,
            Route::Refuse(reason) => panic!("refused: {reason:?}"),
        }
    }

    #[test]
    fn local_choice_follows_the_fallback_table() {
        let table = [
            (when(READY, ACCOUNT, "fr"), local(None)),
            (when(READY, NO_ACCOUNT, "fr"), local(None)),
            (when(READY, ACCOUNT, "auto"), local(None)),
            (when(READY, NO_ACCOUNT, "auto"), local(None)),
            (when(NOT_READY, ACCOUNT, "fr"), cloud(Some(ModelNotReady))),
            (
                when(NOT_READY, NO_ACCOUNT, "fr"),
                Route::Refuse(ModelNotReady),
            ),
            (when(NOT_READY, ACCOUNT, "auto"), cloud(Some(ModelNotReady))),
            (
                when(READY, ACCOUNT, "ja"),
                cloud(Some(LanguageNotSupported)),
            ),
            (
                when(READY, NO_ACCOUNT, "ja"),
                Route::Refuse(LanguageNotSupported),
            ),
            // The language is the lasting reason, so it is the one reported.
            (
                when(NOT_READY, ACCOUNT, "ja"),
                cloud(Some(LanguageNotSupported)),
            ),
            (
                when(NOT_READY, NO_ACCOUNT, "ja"),
                Route::Refuse(LanguageNotSupported),
            ),
        ];
        for (conditions, expected) in table {
            assert_eq!(route(Engine::Local, conditions), expected, "{conditions:?}");
        }
    }

    #[test]
    fn cloud_choice_always_goes_to_the_cloud_first() {
        for model_ready in [READY, NOT_READY] {
            for signed_in in [ACCOUNT, NO_ACCOUNT] {
                for language in ["auto", "fr", "ja"] {
                    let conditions = when(model_ready, signed_in, language);
                    assert_eq!(
                        route(Engine::Cloud, conditions),
                        cloud(None),
                        "{conditions:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_failed_local_run_goes_to_the_cloud_once_or_is_refused_without_an_account() {
        let failed = used(local(None));
        assert_eq!(
            reroute(failed, FailureKind::Other, when(READY, ACCOUNT, "fr")),
            Some(cloud(Some(LocalFailed)))
        );
        assert_eq!(
            reroute(failed, FailureKind::Other, when(READY, NO_ACCOUNT, "fr")),
            Some(Route::Refuse(LocalFailed))
        );
    }

    #[test]
    fn a_refused_cloud_request_goes_to_a_ready_local_model() {
        let failed = used(cloud(None));
        assert_eq!(
            reroute(failed, FailureKind::RateLimited, when(READY, ACCOUNT, "fr")),
            Some(local(Some(CloudLimitReached)))
        );
        for kind in [
            FailureKind::Network,
            FailureKind::Timeout,
            FailureKind::ServerError,
            FailureKind::SessionExpired,
            FailureKind::SignedOut,
            FailureKind::TooLarge,
            FailureKind::Rejected,
            FailureKind::Other,
        ] {
            assert_eq!(
                reroute(failed, kind, when(READY, NO_ACCOUNT, "auto")),
                Some(local(Some(CloudFailed))),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_cloud_error_stands_when_the_local_model_cannot_serve() {
        let failed = used(cloud(None));
        for conditions in [
            when(NOT_READY, ACCOUNT, "fr"),
            when(NOT_READY, NO_ACCOUNT, "auto"),
            when(READY, ACCOUNT, "ja"),
        ] {
            assert_eq!(
                reroute(failed, FailureKind::RateLimited, conditions),
                None,
                "{conditions:?}"
            );
        }
    }

    #[test]
    fn a_fallback_is_never_rerouted() {
        let conditions = when(READY, ACCOUNT, "fr");
        for failed in [
            used(cloud(Some(ModelNotReady))),
            used(cloud(Some(LocalFailed))),
            used(cloud(Some(LanguageNotSupported))),
            used(local(Some(CloudLimitReached))),
            used(local(Some(CloudFailed))),
        ] {
            assert_eq!(
                reroute(failed, FailureKind::Other, conditions),
                None,
                "{failed:?}"
            );
        }
    }

    #[test]
    fn engines_and_reasons_round_trip_through_their_history_names() {
        for engine in [Engine::Local, Engine::Cloud] {
            assert_eq!(Engine::parse(engine.as_str()), Some(engine));
        }
        for reason in [
            ModelNotReady,
            LocalFailed,
            LanguageNotSupported,
            CloudLimitReached,
            CloudFailed,
        ] {
            assert_eq!(FallbackReason::parse(reason.as_str()), Some(reason));
            // History stores the same names the frontend receives.
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::json!(reason.as_str())
            );
        }
        assert_eq!(
            serde_json::to_value(Engine::Local).unwrap(),
            serde_json::json!("local")
        );
        assert_eq!(Engine::parse("other"), None);
        assert_eq!(FallbackReason::parse("other"), None);
    }
}
