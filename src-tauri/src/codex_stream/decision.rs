//! Whether a dictation's text comes from its stream or from the upload. The
//! rules are in one function over plain facts, in the order they apply.

use std::fmt;

use super::health::Setback;
use super::transcript::finish_sentence;

/// Why a dictation has no stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotOpened {
    /// The recording stopped before anything decided.
    Undecided,
    SettingOff,
    OtherEngine,
    SignedOut,
    Health(Setback),
}

/// How a stream went wrong by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No session to open it with: signed out meanwhile, or a refresh failed.
    NoSession,
    /// The handshake got this HTTP status instead of 101.
    Refused(u16),
    /// No connection, or no handshake over it.
    Connect,
    TranscriptFailed,
    /// Fatal, or before the session started.
    SessionError,
    /// A frame that is not an event.
    Unreadable,
    /// The socket closed before the session did.
    ClosedEarly,
    /// The session closed with speech it had not finished transcribing.
    Unfinished,
}

impl Failure {
    /// A 401 is a token to refresh, which the upload does: it is no refusal
    /// of the stream itself.
    fn setback(self) -> Setback {
        match self {
            Self::Refused(status) if status != 401 => Setback::Refused,
            _ => Setback::Failed,
        }
    }
}

/// Where the stream stands once the recording stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Not asked yet.
    Unknown,
    /// Still no `session.started` after the wait for it.
    NotStarted,
    /// Too much audio was still unsent when the recording stopped, or what
    /// was left did not go out in time.
    Lagging,
    /// Still no closed session after the wait that follows `session.close`.
    NotClosed,
    Failed(Failure),
    /// Closed with every utterance final: the text, trimmed, maybe empty.
    Closed(String),
}

pub(crate) struct Facts {
    pub(crate) not_opened: Option<NotOpened>,
    pub(crate) cancelled: bool,
    /// Quiet speech was raised for the engine, so the stream heard another
    /// level than the upload would carry.
    pub(crate) audio_raised: bool,
    /// The language at the stop is not the one the stream was started in.
    pub(crate) language_changed: bool,
    pub(crate) stream: Standing,
}

/// Why the upload transcribes a dictation that had, or could have had, a
/// stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    NotOpened(NotOpened),
    Cancelled,
    QuietSpeech,
    LanguageChanged,
    NotStarted,
    Lagging,
    NotClosed,
    Failed(Failure),
    Empty,
}

impl Skip {
    /// What the skip teaches the next dictations.
    pub(crate) fn setback(self) -> Option<Setback> {
        match self {
            Self::NotStarted | Self::Lagging | Self::NotClosed => Some(Setback::Failed),
            Self::Failed(failure) => Some(failure.setback()),
            Self::NotOpened(_)
            | Self::Cancelled
            | Self::QuietSpeech
            | Self::LanguageChanged
            | Self::Empty => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Nothing rules the stream out yet: ask where it stands, then decide
    /// again.
    Wait,
    Skip(Skip),
    Use(String),
}

/// The first rule that applies wins.
pub(crate) fn decide(facts: &Facts) -> Decision {
    if let Some(reason) = facts.not_opened {
        return Decision::Skip(Skip::NotOpened(reason));
    }
    if facts.cancelled {
        return Decision::Skip(Skip::Cancelled);
    }
    if facts.audio_raised {
        return Decision::Skip(Skip::QuietSpeech);
    }
    if facts.language_changed {
        return Decision::Skip(Skip::LanguageChanged);
    }
    match &facts.stream {
        Standing::Unknown => Decision::Wait,
        Standing::NotStarted => Decision::Skip(Skip::NotStarted),
        Standing::Lagging => Decision::Skip(Skip::Lagging),
        Standing::NotClosed => Decision::Skip(Skip::NotClosed),
        Standing::Failed(failure) => Decision::Skip(Skip::Failed(*failure)),
        // A silent clip: the upload says so, as it always did.
        Standing::Closed(text) if text.is_empty() => Decision::Skip(Skip::Empty),
        Standing::Closed(text) => Decision::Use(finish_sentence(text)),
    }
}

impl fmt::Display for NotOpened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undecided => f.write_str("the recording stopped before it was decided"),
            Self::SettingOff => f.write_str("the setting is off"),
            Self::OtherEngine => f.write_str("another engine was to transcribe"),
            Self::SignedOut => f.write_str("signed out"),
            Self::Health(Setback::Refused) => f.write_str("the server refused a stream lately"),
            Self::Health(Setback::Failed) => f.write_str("a stream failed lately"),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSession => f.write_str("no session"),
            Self::Refused(status) => write!(f, "handshake refused with HTTP {status}"),
            Self::Connect => f.write_str("no connection"),
            Self::TranscriptFailed => f.write_str("transcript.failed"),
            Self::SessionError => f.write_str("session.error"),
            Self::Unreadable => f.write_str("unreadable event"),
            Self::ClosedEarly => f.write_str("socket closed early"),
            Self::Unfinished => f.write_str("speech left unfinished"),
        }
    }
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOpened(reason) => write!(f, "not opened ({reason})"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::QuietSpeech => f.write_str("quiet speech was raised"),
            Self::LanguageChanged => f.write_str("the language changed"),
            Self::NotStarted => f.write_str("the session did not start in time"),
            Self::Lagging => f.write_str("too much audio was unsent"),
            Self::NotClosed => f.write_str("the session did not close in time"),
            Self::Failed(failure) => write!(f, "failed ({failure})"),
            Self::Empty => f.write_str("empty text"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream that gave "Bonjour à tous" and that nothing rules out.
    fn usable() -> Facts {
        Facts {
            not_opened: None,
            cancelled: false,
            audio_raised: false,
            language_changed: false,
            stream: Standing::Closed("Bonjour à tous".to_string()),
        }
    }

    fn skip(skip: Skip) -> Decision {
        Decision::Skip(skip)
    }

    #[test]
    fn each_rule_decides_and_teaches_what_it_should() {
        let failed = |failure| Facts {
            stream: Standing::Failed(failure),
            ..usable()
        };
        // (facts, decision, what the next dictations remember)
        let rows = [
            (
                Facts {
                    not_opened: Some(NotOpened::SettingOff),
                    ..usable()
                },
                skip(Skip::NotOpened(NotOpened::SettingOff)),
                None,
            ),
            (
                Facts {
                    cancelled: true,
                    ..usable()
                },
                skip(Skip::Cancelled),
                None,
            ),
            (
                Facts {
                    audio_raised: true,
                    ..usable()
                },
                skip(Skip::QuietSpeech),
                None,
            ),
            (
                Facts {
                    language_changed: true,
                    ..usable()
                },
                skip(Skip::LanguageChanged),
                None,
            ),
            (
                Facts {
                    stream: Standing::NotStarted,
                    ..usable()
                },
                skip(Skip::NotStarted),
                Some(Setback::Failed),
            ),
            (
                Facts {
                    stream: Standing::Lagging,
                    ..usable()
                },
                skip(Skip::Lagging),
                Some(Setback::Failed),
            ),
            (
                Facts {
                    stream: Standing::NotClosed,
                    ..usable()
                },
                skip(Skip::NotClosed),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::TranscriptFailed),
                skip(Skip::Failed(Failure::TranscriptFailed)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::SessionError),
                skip(Skip::Failed(Failure::SessionError)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::Unreadable),
                skip(Skip::Failed(Failure::Unreadable)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::ClosedEarly),
                skip(Skip::Failed(Failure::ClosedEarly)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::Unfinished),
                skip(Skip::Failed(Failure::Unfinished)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::Connect),
                skip(Skip::Failed(Failure::Connect)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::NoSession),
                skip(Skip::Failed(Failure::NoSession)),
                Some(Setback::Failed),
            ),
            (
                failed(Failure::Refused(403)),
                skip(Skip::Failed(Failure::Refused(403))),
                Some(Setback::Refused),
            ),
            (
                failed(Failure::Refused(401)),
                skip(Skip::Failed(Failure::Refused(401))),
                Some(Setback::Failed),
            ),
            (
                Facts {
                    stream: Standing::Closed(String::new()),
                    ..usable()
                },
                skip(Skip::Empty),
                None,
            ),
            (usable(), Decision::Use("Bonjour à tous.".to_string()), None),
        ];
        for (index, (facts, decision, setback)) in rows.into_iter().enumerate() {
            assert_eq!(decide(&facts), decision, "row {index}");
            if let Decision::Skip(skip) = decision {
                assert_eq!(skip.setback(), setback, "setback of row {index}");
            }
        }
    }

    #[test]
    fn the_first_rule_that_applies_wins() {
        let everything_at_once = Facts {
            not_opened: Some(NotOpened::SignedOut),
            cancelled: true,
            audio_raised: true,
            language_changed: true,
            stream: Standing::Failed(Failure::Refused(403)),
        };
        let order = [
            Skip::NotOpened(NotOpened::SignedOut),
            Skip::Cancelled,
            Skip::QuietSpeech,
            Skip::LanguageChanged,
            Skip::Failed(Failure::Refused(403)),
        ];
        let mut facts = everything_at_once;
        for (index, expected) in order.into_iter().enumerate() {
            assert_eq!(decide(&facts), skip(expected), "rule {}", index + 1);
            match index {
                0 => facts.not_opened = None,
                1 => facts.cancelled = false,
                2 => facts.audio_raised = false,
                _ => facts.language_changed = false,
            }
        }
    }

    #[test]
    fn a_stream_nothing_rules_out_is_waited_for() {
        let mut facts = Facts {
            stream: Standing::Unknown,
            ..usable()
        };
        assert_eq!(decide(&facts), Decision::Wait);

        // A cancel ends the wait: the stream is no longer asked.
        facts.cancelled = true;
        assert_eq!(decide(&facts), skip(Skip::Cancelled));
    }

    #[test]
    fn the_log_names_each_reason_without_any_text() {
        assert_eq!(
            Skip::Failed(Failure::Refused(403)).to_string(),
            "failed (handshake refused with HTTP 403)"
        );
        assert_eq!(
            Skip::NotOpened(NotOpened::Health(Setback::Refused)).to_string(),
            "not opened (the server refused a stream lately)"
        );
        assert_eq!(Skip::QuietSpeech.to_string(), "quiet speech was raised");
    }
}
