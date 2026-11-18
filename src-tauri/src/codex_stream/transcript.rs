//! The text of one dictation session, built from its transcript events the
//! way the official client builds it. No I/O and no clock.

#[derive(Debug, Default)]
pub(crate) struct Transcript {
    /// In order of first appearance.
    utterances: Vec<Utterance>,
}

#[derive(Debug)]
struct Utterance {
    id: String,
    partial: Option<Revised>,
    finished: Option<Revised>,
}

#[derive(Debug)]
struct Revised {
    revision: u64,
    text: String,
}

impl Transcript {
    /// An utterance takes its place in the text when it is first named.
    pub(crate) fn heard(&mut self, utterance_id: &str) {
        self.utterance(utterance_id);
    }

    /// The text of an utterance so far. Kept only while the utterance has no
    /// final text, and never over a later revision.
    pub(crate) fn segment(&mut self, utterance_id: &str, revision: u64, text: &str) {
        let utterance = self.utterance(utterance_id);
        let older = |kept: &Revised| revision < kept.revision;
        if utterance.finished.is_some() || utterance.partial.as_ref().is_some_and(older) {
            return;
        }
        utterance.partial = Some(Revised::new(revision, text));
    }

    /// The final text of an utterance, never over a later revision of it.
    pub(crate) fn finish(&mut self, utterance_id: &str, revision: u64, text: &str) {
        let utterance = self.utterance(utterance_id);
        utterance.partial = None;
        let older = |kept: &Revised| revision < kept.revision;
        if !utterance.finished.as_ref().is_some_and(older) {
            utterance.finished = Some(Revised::new(revision, text));
        }
    }

    /// The final texts joined with one space. `None` while an utterance has
    /// a partial text and no final one: its speech was not all transcribed.
    pub(crate) fn final_text(&self) -> Option<String> {
        let unfinished =
            |utterance: &Utterance| utterance.partial.is_some() && utterance.finished.is_none();
        if self.utterances.iter().any(unfinished) {
            return None;
        }
        let finals: Vec<&str> = self
            .utterances
            .iter()
            .filter_map(|utterance| utterance.finished.as_ref())
            .map(|finished| finished.text.as_str())
            .collect();
        Some(finals.join(" ").trim().to_string())
    }

    /// What is known of the text now: the final text of each utterance, or
    /// its partial text until then.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "for the recording overlay, which does not show it yet"
        )
    )]
    pub(crate) fn so_far(&self) -> String {
        let texts: Vec<&str> = self
            .utterances
            .iter()
            .filter_map(|utterance| utterance.finished.as_ref().or(utterance.partial.as_ref()))
            .map(|revised| revised.text.trim())
            .filter(|text| !text.is_empty())
            .collect();
        texts.join(" ")
    }

    fn utterance(&mut self, id: &str) -> &mut Utterance {
        let index = match self.utterances.iter().position(|known| known.id == id) {
            Some(index) => index,
            None => {
                self.utterances.push(Utterance {
                    id: id.to_string(),
                    partial: None,
                    finished: None,
                });
                self.utterances.len() - 1
            }
        };
        &mut self.utterances[index]
    }
}

impl Revised {
    fn new(revision: u64, text: &str) -> Self {
        Self {
            revision,
            text: text.to_string(),
        }
    }
}

/// Letters and digits of the Latin, Greek and Cyrillic scripts: Basic Latin
/// to Latin Extended-B, Greek and Coptic, Cyrillic and its supplement, Latin
/// Extended Additional, Greek Extended. The punctuation inside these blocks
/// is left out by the letter-or-digit test.
const WORD_SCRIPTS: [(char, char); 5] = [
    ('\u{0030}', '\u{024F}'),
    ('\u{0370}', '\u{03FF}'),
    ('\u{0400}', '\u{052F}'),
    ('\u{1E00}', '\u{1EFF}'),
    ('\u{1F00}', '\u{1FFF}'),
];

/// The stream leaves out the closing mark of a long dictation, which the
/// upload never does: a text that ends on a word gets a full stop. Only the
/// scripts this was measured on are completed.
pub(crate) fn finish_sentence(text: &str) -> String {
    let text = text.trim();
    let ends_a_word = text.chars().next_back().is_some_and(|last| {
        last.is_alphanumeric()
            && WORD_SCRIPTS
                .iter()
                .any(|(first, end)| (*first..=*end).contains(&last))
    });
    if ends_a_word {
        format!("{text}.")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Event {
        Heard(&'static str),
        Segment(&'static str, u64, &'static str),
        Final(&'static str, u64, &'static str),
    }
    use Event::{Final, Heard, Segment};

    fn transcript(events: &[Event]) -> Transcript {
        let mut transcript = Transcript::default();
        for event in events {
            match *event {
                Heard(id) => transcript.heard(id),
                Segment(id, revision, text) => transcript.segment(id, revision, text),
                Final(id, revision, text) => transcript.finish(id, revision, text),
            }
        }
        transcript
    }

    #[test]
    fn the_text_follows_the_revision_rules() {
        // (events, final text or None while unfinished, text so far)
        let rows: [(&[Event], Option<&str>, &str); 12] = [
            (&[], Some(""), ""),
            (&[Heard("a")], Some(""), ""),
            (&[Final("a", 1, "Bonjour.")], Some("Bonjour."), "Bonjour."),
            (&[Segment("a", 1, " Bon")], None, "Bon"),
            (
                &[Segment("a", 1, " Bon"), Segment("a", 2, " Bonjour")],
                None,
                "Bonjour",
            ),
            (
                &[Segment("a", 2, " Bonjour"), Segment("a", 1, " Bon")],
                None,
                "Bonjour",
            ),
            (
                &[Segment("a", 2, " Bonjour"), Segment("a", 2, " Bonjour à")],
                None,
                "Bonjour à",
            ),
            (
                &[Segment("a", 5, " Bonjour"), Final("a", 1, "Bonjour.")],
                Some("Bonjour."),
                "Bonjour.",
            ),
            (
                &[Final("a", 3, "Bonjour."), Segment("a", 9, " Bonjour à")],
                Some("Bonjour."),
                "Bonjour.",
            ),
            (
                &[Final("a", 3, "Bonjour."), Final("a", 2, "Bon.")],
                Some("Bonjour."),
                "Bonjour.",
            ),
            (
                &[Final("a", 3, "Bonjour."), Final("a", 3, "Bonjour à tous.")],
                Some("Bonjour à tous."),
                "Bonjour à tous.",
            ),
            (
                &[Final("a", 1, "  Bonjour. "), Final("b", 1, " ")],
                Some("Bonjour."),
                "Bonjour.",
            ),
        ];
        for (index, (events, final_text, so_far)) in rows.iter().enumerate() {
            let transcript = transcript(events);
            assert_eq!(
                transcript.final_text().as_deref(),
                *final_text,
                "final text of row {index}"
            );
            assert_eq!(transcript.so_far(), *so_far, "text so far of row {index}");
        }
    }

    #[test]
    fn utterances_keep_the_order_they_first_appeared_in() {
        let spoken = transcript(&[
            Heard("a"),
            Segment("b", 1, " deux"),
            Final("b", 2, "Deux."),
            Final("c", 1, "Trois."),
            Final("a", 1, "Un."),
        ]);
        assert_eq!(spoken.final_text().as_deref(), Some("Un. Deux. Trois."));
        assert_eq!(spoken.so_far(), "Un. Deux. Trois.");
    }

    #[test]
    fn one_utterance_left_with_a_partial_makes_the_whole_text_unfinished() {
        let spoken = transcript(&[
            Final("a", 1, "Un."),
            Segment("b", 1, " deux"),
            Final("c", 1, "Trois."),
        ]);
        assert_eq!(spoken.final_text(), None);
        assert_eq!(spoken.so_far(), "Un. deux Trois.");
    }

    #[test]
    fn a_text_that_ends_on_a_word_gets_a_full_stop() {
        let rows = [
            ("Bonjour à tous", "Bonjour à tous."),
            ("It costs 42", "It costs 42."),
            ("Fertig mit dem Maß", "Fertig mit dem Maß."),
            ("Καλημέρα κόσμε", "Καλημέρα κόσμε."),
            ("Доброе утро", "Доброе утро."),
            ("  Bonjour à tous \n", "Bonjour à tous."),
            ("Bonjour.", "Bonjour."),
            ("Vraiment ?", "Vraiment ?"),
            ("Enfin !", "Enfin !"),
            ("Et alors…", "Et alors…"),
            ("He said “fine”", "He said “fine”"),
            ("Il a dit « oui »", "Il a dit « oui »"),
            ("See the note (below)", "See the note (below)"),
            ("See [1]", "See [1]"),
            ("Τι;", "Τι;"),
            ("今日は晴れです", "今日は晴れです"),
            ("مرحبا بالعالم", "مرحبا بالعالم"),
            ("", ""),
            ("   ", ""),
        ];
        for (text, finished) in rows {
            assert_eq!(finish_sentence(text), finished, "{text:?}");
        }
    }
}
