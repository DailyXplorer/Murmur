//! Full-text search over meetings and dictations: the FTS5 query a user's
//! words become, and the highlighted text a hit shows. The SQL lives with
//! each database (`meetings::store::search`, `HistoryManager::search`).

use serde::Serialize;
use specta::Type;

/// Results a search returns at most.
pub const SEARCH_LIMIT: usize = 50;

/// Where `snippet()` and `highlight()` open and close a hit. Control
/// characters never appear in transcripts, so they cannot be confused with
/// text, and they never cross IPC: `Snippet::from_marked` strips them.
pub const HIT_START: char = '\u{1}';
pub const HIT_END: char = '\u{2}';

/// Text with the parts that matched. `ranges` are `[start, end)` offsets in
/// UTF-16 code units, the unit JavaScript strings index by.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct Snippet {
    pub text: String,
    pub ranges: Vec<(u32, u32)>,
}

impl Snippet {
    /// Reads text marked with `HIT_START` and `HIT_END`.
    pub fn from_marked(marked: &str) -> Self {
        let mut text = String::with_capacity(marked.len());
        let mut ranges = Vec::new();
        let mut at = 0u32;
        let mut open = None;
        for character in marked.chars() {
            match character {
                HIT_START => open = Some(at),
                HIT_END => {
                    if let Some(start) = open.take() {
                        if start < at {
                            ranges.push((start, at));
                        }
                    }
                }
                _ => {
                    text.push(character);
                    at += character.len_utf16() as u32;
                }
            }
        }
        Self { text, ranges }
    }
}

/// Logs a failed search with its SQLite error code only: the message could
/// quote the query, which is the user's text.
pub fn search_failed(scope: &str, error: &anyhow::Error) -> String {
    let code = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<rusqlite::Error>())
        .and_then(rusqlite::Error::sqlite_error_code);
    log::error!("{scope} search failed: {code:?}");
    "Search failed".to_string()
}

/// The FTS5 query matching a row that holds every term of what the user
/// typed (see `fts_terms`), or `None` when there is nothing to search.
pub fn fts_query(user: &str) -> Option<String> {
    let terms = fts_terms(user);
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// What the user typed as FTS5 terms, each one valid on its own: a word
/// matches as a prefix (`pric` finds "pricing"), words in double quotes as
/// a phrase. Each piece is quoted, so no input is FTS5 syntax. Empty when
/// there is nothing to search (under 2 characters, or no letter or digit).
pub fn fts_terms(user: &str) -> Vec<String> {
    let user = user.trim();
    if user.chars().count() < 2 {
        return Vec::new();
    }
    let mut parts = Vec::new();
    for (index, piece) in user.split('"').enumerate() {
        // Odd pieces sat between quotes; an unclosed quote runs to the end.
        if index % 2 == 1 {
            if let Some(phrase) = quoted(piece) {
                parts.push(phrase);
            }
        } else {
            parts.extend(
                piece
                    .split_whitespace()
                    .filter_map(quoted)
                    .map(|word| word + "*"),
            );
        }
    }
    parts
}

/// `text` as an FTS5 string, or `None` when it holds nothing the tokenizer
/// keeps (an empty string is an FTS5 syntax error).
fn quoted(text: &str) -> Option<String> {
    text.chars()
        .any(char::is_alphanumeric)
        .then(|| format!("\"{}\"", text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_escapes_syntax() {
        assert_eq!(fts_query("a"), None);
        assert_eq!(fts_query("  é "), None);
        assert_eq!(fts_query("--"), None);
        assert_eq!(fts_query("pric"), Some("\"pric\"*".into()));
        assert_eq!(
            fts_query("budget NEAR team"),
            Some("\"budget\"* \"NEAR\"* \"team\"*".into())
        );
        assert_eq!(
            fts_query("-draft OR col:x ^start (a) b*"),
            Some("\"-draft\"* \"OR\"* \"col:x\"* \"^start\"* \"(a)\"* \"b*\"*".into())
        );
        assert_eq!(
            fts_query("roadmap \"next quarter\" q3"),
            Some("\"roadmap\"* \"next quarter\" \"q3\"*".into())
        );
        assert_eq!(
            fts_query("say \"unclosed phrase"),
            Some("\"say\"* \"unclosed phrase\"".into())
        );
        assert_eq!(fts_query("\"\" * -"), None);
        assert_eq!(
            fts_terms("roadmap \"next quarter\""),
            ["\"roadmap\"*", "\"next quarter\""]
        );

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING fts5(x, tokenize='unicode61 remove_diacritics 2');
             INSERT INTO t VALUES ('the pricing of the next quarter');",
        )
        .unwrap();
        for input in [
            "pric",
            "NEAR(a b)",
            "\"next quarter\"",
            "AND OR NOT",
            "x:y",
            "a\"b\"c\"",
            "* ^ - + ( ) { } :",
            "l'été",
        ] {
            let Some(query) = fts_query(input) else {
                continue;
            };
            conn.query_row("SELECT count(*) FROM t WHERE t MATCH ?1", [&query], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_else(|error| panic!("{input:?} -> {query:?}: {error}"));
        }
    }
}
