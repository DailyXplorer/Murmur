//! Which engine transcribes a dictation: Parakeet on this Mac or Codex in the
//! cloud. `route` decides before any audio is sent, and `reroute` decides once
//! after the chosen engine failed. Both are pure, so the whole table is tested.

pub mod local;
pub mod parakeet_languages;

use crate::transcription_error::{FailureKind, TranscriptionError};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// Parakeet TDT 0.6B v3 on this Mac, what a new install starts with.
    #[default]
    Local,
    /// Codex through the user's ChatGPT session.
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
        Engine::Cloud => local_can_serve(conditions.model_ready, conditions.language).then(|| {
            Route::Run(EngineUsed {
                engine: Engine::Local,
                fallback: Some(if kind == FailureKind::RateLimited {
                    FallbackReason::CloudLimitReached
                } else {
                    FallbackReason::CloudFailed
                }),
            })
        }),
    }
}

/// Whether the local model can take a dictation in `language`, chosen or as
/// the fallback of a failed cloud request.
pub fn local_can_serve(model_ready: bool, language: &str) -> bool {
    model_ready && parakeet_languages::supports(language)
}

/// Local cannot serve and no account can: the message says why, since a
/// ChatGPT account is optional.
fn refused(reason: FallbackReason) -> anyhow::Error {
    let kind = match reason {
        FallbackReason::ModelNotReady => FailureKind::LocalNotReady,
        FallbackReason::LocalFailed => FailureKind::LocalFailed,
        FallbackReason::LanguageNotSupported => FailureKind::LanguageNeedsCloud,
        FallbackReason::CloudLimitReached | FallbackReason::CloudFailed => FailureKind::Other,
    };
    TranscriptionError::new(
        kind,
        format!("Local transcription unavailable ({reason:?}) and no ChatGPT account"),
    )
    .into()
}

/// Transcribes with the engine `route` picks, then once with the one
/// `reroute` picks if that failed. `run` transcribes with one engine. An
/// error once `cancelled` stands as it is.
pub async fn transcribe<F, Fut>(
    choice: Engine,
    conditions: Conditions<'_>,
    cancelled: impl Fn() -> bool,
    mut run: F,
) -> anyhow::Result<(String, EngineUsed)>
where
    F: FnMut(Engine) -> Fut,
    Fut: Future<Output = anyhow::Result<String>>,
{
    let source = match route(choice, conditions) {
        Route::Run(source) => source,
        Route::Refuse(reason) => return Err(refused(reason)),
    };
    let error = match run(source.engine).await {
        Ok(text) => return Ok((text, source)),
        Err(error) => error,
    };
    if cancelled() {
        return Err(error);
    }
    match reroute(source, TranscriptionError::kind_of(&error), conditions) {
        None => Err(error),
        Some(Route::Refuse(reason)) => {
            log::warn!("Local transcription failed: {error:#}");
            Err(refused(reason))
        }
        Some(Route::Run(fallback)) => {
            log::warn!(
                "Transcribing with the {} engine instead ({:?}): {error:#}",
                fallback.engine.as_str(),
                fallback.fallback
            );
            Ok((run(fallback.engine).await?, fallback))
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

    /// Runs a dictation where each engine answers as given, and returns the
    /// outcome with the engines asked, in order.
    fn dictate(
        choice: Engine,
        conditions: Conditions,
        local: Result<&str, FailureKind>,
        cloud: Result<&str, FailureKind>,
    ) -> (anyhow::Result<(String, EngineUsed)>, Vec<Engine>) {
        let mut asked = Vec::new();
        let result = tauri::async_runtime::block_on(transcribe(
            choice,
            conditions,
            || false,
            |engine| {
                asked.push(engine);
                let answer = match engine {
                    Engine::Local => local,
                    Engine::Cloud => cloud,
                };
                async move {
                    answer
                        .map(str::to_string)
                        .map_err(|kind| TranscriptionError::new(kind, "fake").into())
                }
            },
        ));
        (result, asked)
    }

    #[test]
    fn a_local_failure_is_transcribed_in_the_cloud_with_an_account() {
        let (result, asked) = dictate(
            Engine::Local,
            when(READY, ACCOUNT, "fr"),
            Err(FailureKind::Other),
            Ok("Bonjour"),
        );
        let (text, source) = result.unwrap();
        assert_eq!(text, "Bonjour");
        assert_eq!(source, used(cloud(Some(LocalFailed))));
        assert_eq!(asked, [Engine::Local, Engine::Cloud]);
    }

    #[test]
    fn a_local_failure_without_an_account_says_local_failed() {
        let (result, asked) = dictate(
            Engine::Local,
            when(READY, NO_ACCOUNT, "fr"),
            Err(FailureKind::Other),
            Ok("never"),
        );
        let error = result.unwrap_err();
        assert_eq!(
            TranscriptionError::kind_of(&error),
            FailureKind::LocalFailed
        );
        assert_eq!(asked, [Engine::Local]);
    }

    #[test]
    fn a_refused_route_sends_no_audio_and_says_why() {
        for (conditions, kind) in [
            (
                when(NOT_READY, NO_ACCOUNT, "fr"),
                FailureKind::LocalNotReady,
            ),
            (
                when(READY, NO_ACCOUNT, "ja"),
                FailureKind::LanguageNeedsCloud,
            ),
        ] {
            let (result, asked) = dictate(Engine::Local, conditions, Ok("never"), Ok("never"));
            assert_eq!(TranscriptionError::kind_of(&result.unwrap_err()), kind);
            assert!(asked.is_empty());
        }
    }

    #[test]
    fn a_cloud_limit_is_transcribed_on_this_mac_once() {
        let (result, asked) = dictate(
            Engine::Cloud,
            when(READY, ACCOUNT, "auto"),
            Ok("Salut"),
            Err(FailureKind::RateLimited),
        );
        let (text, source) = result.unwrap();
        assert_eq!(text, "Salut");
        assert_eq!(source, used(local(Some(CloudLimitReached))));
        assert_eq!(asked, [Engine::Cloud, Engine::Local]);

        // A failing fallback is the error, with no third try.
        let (result, asked) = dictate(
            Engine::Cloud,
            when(READY, ACCOUNT, "auto"),
            Err(FailureKind::Other),
            Err(FailureKind::RateLimited),
        );
        assert!(result.is_err());
        assert_eq!(asked, [Engine::Cloud, Engine::Local]);
    }

    #[test]
    fn a_cancelled_dictation_is_not_sent_again() {
        let mut asked = 0;
        let result = tauri::async_runtime::block_on(transcribe(
            Engine::Local,
            when(READY, ACCOUNT, "fr"),
            || true,
            |_| {
                asked += 1;
                async { Err::<String, _>(anyhow::anyhow!("cancelled")) }
            },
        ));
        assert!(result.is_err());
        assert_eq!(asked, 1);
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
