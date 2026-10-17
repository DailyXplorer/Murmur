//! The Markdown of a meeting's transcript. Copy and Export both use it, so
//! the copied and the saved text are always the same.

use chrono::{DateTime, FixedOffset};
use std::collections::HashMap;
use std::fmt::Write;

use super::merge::PLACEHOLDER;
use super::store::{MeetingSpeaker, MeetingTurn};

/// File names stay short enough for every file system and dialog.
const MAX_FILE_STEM_CHARS: usize = 80;

pub struct Transcript<'a> {
    pub title: Option<&'a str>,
    /// The start, in the time zone the reader expects.
    pub started_at: DateTime<FixedOffset>,
    pub duration_ms: i64,
    pub speakers: &'a [MeetingSpeaker],
    pub turns: &'a [MeetingTurn],
}

/// `04:07` under an hour, `1:02:09` from one hour on, like the app.
pub fn clock(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub fn default_title(started_at: &DateTime<FixedOffset>) -> String {
    format!("Meeting · {}", started_at.format("%-d %b %Y"))
}

/// Keeps names from turning into Markdown emphasis or links.
fn escape(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '\\' | '*' | '_' | '[' | ']' | '`' | '#') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

pub fn transcript_markdown(transcript: &Transcript) -> String {
    let names: HashMap<&str, &str> = transcript
        .speakers
        .iter()
        .map(|speaker| (speaker.key.as_str(), speaker.name.as_str()))
        .collect();
    let mut speaking: Vec<&str> = transcript
        .turns
        .iter()
        .map(|turn| turn.speaker_key.as_str())
        .filter(|key| *key != PLACEHOLDER)
        .collect();
    speaking.sort_unstable();
    speaking.dedup();

    let title = transcript
        .title
        .map(str::to_string)
        .unwrap_or_else(|| default_title(&transcript.started_at));
    let mut out = String::new();
    let _ = writeln!(out, "# {}", escape(&title));
    let _ = writeln!(
        out,
        "{} · {} · {} speaker{}",
        transcript.started_at.format("%-d %b %Y · %H:%M"),
        clock(transcript.duration_ms),
        speaking.len(),
        if speaking.len() == 1 { "" } else { "s" }
    );
    out.push_str("\n## Transcript\n");

    let mut previous: Option<&str> = None;
    for turn in transcript.turns {
        if turn.speaker_key == PLACEHOLDER {
            let _ = write!(
                out,
                "\n*[Part {}–{} could not be transcribed]*\n",
                clock(turn.start_ms),
                clock(turn.end_ms)
            );
            previous = None;
            continue;
        }
        if previous != Some(turn.speaker_key.as_str()) {
            let name = names
                .get(turn.speaker_key.as_str())
                .copied()
                .unwrap_or(turn.speaker_key.as_str());
            let _ = write!(out, "\n**{}** ({})\n", escape(name), clock(turn.start_ms));
        } else {
            out.push('\n');
        }
        out.push_str(&turn.text);
        out.push('\n');
        previous = Some(turn.speaker_key.as_str());
    }
    out
}

/// `2026-10-17 Weekly sync.md`, with characters that file systems refuse
/// replaced.
pub fn file_name(started_at: &DateTime<FixedOffset>, title: Option<&str>) -> String {
    let title: String = title
        .unwrap_or("Meeting")
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | ':' | '\\' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let stem = format!("{} {}", started_at.format("%Y-%m-%d"), title.trim());
    let stem: String = stem.chars().take(MAX_FILE_STEM_CHARS).collect();
    format!("{}.md", stem.trim_end_matches(['.', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap()
    }

    fn speaker(key: &str, name: &str) -> MeetingSpeaker {
        MeetingSpeaker {
            key: key.into(),
            name: name.into(),
            default_name: name.into(),
            color_index: 0,
        }
    }

    fn turn(key: &str, start_ms: i64, end_ms: i64, text: &str) -> MeetingTurn {
        MeetingTurn {
            speaker_key: key.into(),
            start_ms,
            end_ms,
            text: text.into(),
            low_confidence: false,
        }
    }

    fn french_meeting() -> (Vec<MeetingSpeaker>, Vec<MeetingTurn>) {
        (
            vec![speaker("me", "Me"), speaker("s1", "Speaker 2")],
            vec![
                turn("me", 12_000, 15_000, "Bonjour à tous, on commence ?"),
                turn("me", 19_000, 22_000, "Premier point : le budget."),
                turn("s1", 41_000, 44_000, "Oui, il est validé."),
                turn(PLACEHOLDER, 600_000, 1_200_000, ""),
                turn(
                    "s1",
                    1_203_000,
                    1_206_000,
                    "Et le planning glisse d’une semaine.",
                ),
                turn("me", 3_700_000, 3_702_000, "Merci, à jeudi."),
            ],
        )
    }

    #[test]
    fn the_transcript_exports_as_golden_markdown() {
        let (speakers, turns) = french_meeting();
        let markdown = transcript_markdown(&Transcript {
            title: Some("Point hebdo"),
            started_at: at("2026-10-17T14:05:00+02:00"),
            duration_ms: 3_732_000,
            speakers: &speakers,
            turns: &turns,
        });
        assert_eq!(
            markdown,
            "# Point hebdo
17 Oct 2026 · 14:05 · 1:02:12 · 2 speakers

## Transcript

**Me** (00:12)
Bonjour à tous, on commence ?

Premier point : le budget.

**Speaker 2** (00:41)
Oui, il est validé.

*[Part 10:00–20:00 could not be transcribed]*

**Speaker 2** (20:03)
Et le planning glisse d’une semaine.

**Me** (1:01:40)
Merci, à jeudi.
"
        );
    }

    #[test]
    fn renamed_speakers_appear_by_their_new_name() {
        let (mut speakers, turns) = french_meeting();
        speakers[1].name = "Sam_K".into();
        let markdown = transcript_markdown(&Transcript {
            title: None,
            started_at: at("2026-03-02T09:30:00Z"),
            duration_ms: 65_000,
            speakers: &speakers,
            turns: &turns[2..3],
        });
        assert_eq!(
            markdown,
            "# Meeting · 2 Mar 2026
2 Mar 2026 · 09:30 · 01:05 · 1 speaker

## Transcript

**Sam\\_K** (00:41)
Oui, il est validé.
"
        );
    }

    #[test]
    fn an_english_meeting_without_turns_still_has_its_header() {
        let markdown = transcript_markdown(&Transcript {
            title: Some("Q4 *planning*"),
            started_at: at("2026-12-01T16:00:00-05:00"),
            duration_ms: 0,
            speakers: &[],
            turns: &[],
        });
        assert_eq!(
            markdown,
            "# Q4 \\*planning\\*\n1 Dec 2026 · 16:00 · 00:00 · 0 speakers\n\n## Transcript\n"
        );
    }

    #[test]
    fn file_names_are_dated_and_safe() {
        let start = at("2026-10-17T14:05:00+02:00");
        assert_eq!(
            file_name(&start, Some("Q4: plan / budget?")),
            "2026-10-17 Q4- plan - budget-.md"
        );
        assert_eq!(file_name(&start, None), "2026-10-17 Meeting.md");
        let long = file_name(&start, Some(&"x".repeat(200)));
        assert_eq!(long.chars().count(), 80 + 3);
        assert_eq!(file_name(&start, Some("Notes...")), "2026-10-17 Notes.md");
    }
}
