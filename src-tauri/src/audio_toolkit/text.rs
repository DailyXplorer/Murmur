use crate::meetings::tokens::is_title;
use natural::phonetics::soundex;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;
use strsim::levenshtein;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Builds an n-gram string by cleaning and concatenating words
///
/// Strips punctuation from each word, lowercases, and joins without spaces.
/// This allows matching "Charge B" against "ChargeBee".
fn build_ngram(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| build_match_key(w))
        .collect::<Vec<_>>()
        .concat()
}

/// The lowercase letters and digits of a word, accents kept, as the
/// everyday-word list is written: "Déjà," gives "déjà".
fn word_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Everyday English and French words, lowercase.
fn is_everyday_word(word: &str) -> bool {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS
        .get_or_init(|| include_str!("common_words.txt").lines().collect())
        .contains(word.to_lowercase().as_str())
}

/// [`word_key`] without accents, so a name compares equal however the
/// transcriber accented it: "Zoé" and "Zoe" both give "zoe".
fn build_match_key(word: &str) -> String {
    word_key(word)
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

struct CustomWordMatchKey {
    word_index: usize,
    key: String,
    /// The [`word_key`] the key was folded from, accents kept.
    spelling: String,
}

fn build_custom_word_match_keys(word: &str, word_index: usize) -> Vec<CustomWordMatchKey> {
    let primary_key = build_match_key(word);
    let mut keys = Vec::with_capacity(2);

    // The fallback matcher is intentionally limited to Latin terms. Its
    // whitespace tokenization is not suitable for CJK scripts, whose custom
    // words are simply skipped by this fallback.
    if is_supported_fuzzy_key(&primary_key) {
        keys.push(CustomWordMatchKey {
            word_index,
            key: primary_key.clone(),
            spelling: word_key(word),
        });
    }

    if word.contains('&') {
        let expanded = word.replace('&', " and ");
        let expanded_key = build_match_key(&expanded);
        if is_supported_fuzzy_key(&expanded_key) && expanded_key != primary_key {
            keys.push(CustomWordMatchKey {
                word_index,
                key: expanded_key,
                spelling: word_key(&expanded),
            });
        }
    }

    keys
}

/// A key of Latin letters and digits. Its accents are already gone, but
/// letters such as "œ" or "ß" remain.
fn is_supported_fuzzy_key(key: &str) -> bool {
    let latin = |c: char| ('\u{c0}'..='\u{24f}').contains(&c) && c.is_alphabetic();
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || latin(c))
}

fn supports_soundex(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic())
}

/// The longest word too short for a reliable Soundex code.
const MAX_SHORT_WORD_LEN: usize = 4;

/// Scores a candidate against one custom-word key: normalized Levenshtein
/// distance, boosted when both sound alike. Lower is closer; 0.0 is exact.
/// `written_as_name` is a single dictated word with a capital letter.
fn match_score(candidate: &str, key: &str, written_as_name: bool) -> f64 {
    let max_len = candidate.chars().count().max(key.chars().count()) as f64;
    let levenshtein_score = if max_len > 0.0 {
        levenshtein(candidate, key) as f64 / max_len
    } else {
        1.0
    };

    // Soundex is an English/ASCII phonetic algorithm. Numeric terms can
    // still use edit distance, but must not receive a phonetic boost. Nor
    // can short words: Soundex keeps their first letter and consonants
    // only, so "lu" sounds like "Lou" and "mille" like "Mila". A short word
    // capitalised mid-sentence is a name the transcriber heard, so "Zack"
    // still becomes "Zach".
    let phonetic_match = supports_soundex(candidate)
        && supports_soundex(key)
        && (written_as_name || candidate.len().min(key.len()) > MAX_SHORT_WORD_LEN)
        && soundex(candidate, key);

    if phonetic_match {
        levenshtein_score * 0.3
    } else {
        levenshtein_score
    }
}

/// Finds the closest custom-word key for a candidate string, if any scores
/// under the threshold.
fn find_best_match<'a>(
    candidate: &str,
    custom_word_match_keys: &'a [CustomWordMatchKey],
    threshold: f64,
    written_as_name: bool,
) -> Option<(&'a CustomWordMatchKey, f64)> {
    if !is_supported_fuzzy_key(candidate) || candidate.chars().count() > 50 {
        return None;
    }

    let mut best_match: Option<(&CustomWordMatchKey, f64)> = None;

    for custom_word_key in custom_word_match_keys {
        // Skip if lengths are too different (optimization + prevents over-matching)
        // Use percentage-based check: max 25% length difference (prevents n-grams from
        // matching significantly shorter custom words, e.g., "openaigpt" vs "openai")
        let candidate_len = candidate.chars().count();
        let custom_word_len = custom_word_key.key.chars().count();
        let len_diff = candidate_len.abs_diff(custom_word_len) as f64;
        let max_len = candidate_len.max(custom_word_len) as f64;
        let max_allowed_diff = (max_len * 0.25).max(2.0);
        if len_diff > max_allowed_diff {
            continue;
        }

        let score = match_score(candidate, &custom_word_key.key, written_as_name);
        if score < threshold && best_match.is_none_or(|(_, best_score)| score < best_score) {
            best_match = Some((custom_word_key, score));
        }
    }

    best_match
}

/// How closely one word of a span matches any part of a custom-word key of
/// about its length: "b" against "chargebee" scores 0.0, "app" against
/// "chatgpt" scores poorly.
fn best_part_score(word: &str, key: &str) -> f64 {
    let key: Vec<char> = key.chars().collect();
    let word_len = word.chars().count();
    if word_len == 0 {
        return 1.0;
    }
    let mut best = 1.0_f64;
    for part_len in word_len.saturating_sub(1).max(1)..=(word_len + 1).min(key.len()) {
        for part in key.windows(part_len) {
            best = best.min(match_score(word, &part.iter().collect::<String>(), false));
        }
    }
    best
}

/// Longest span matched against one custom word. Four covers a spelled-out
/// acronym after a word, as in "Chat G P T".
const MAX_SPAN_WORDS: usize = 4;

/// A span of `len` words starting at word `start` that matches a custom word.
struct SpanMatch<'a> {
    start: usize,
    len: usize,
    replacement: &'a String,
    /// A French elision on the first word, kept before the replacement: the
    /// "l'" of "l'iPhone".
    elision: &'a str,
    /// A possessive or plural ending of the last word, kept after the
    /// replacement: the "'s" of "ChatGPT's", the "s" of "iPhones".
    ending: &'a str,
    score: f64,
}

/// A single word whose capital can only mark a name the transcriber heard.
/// The capital of a sentence opener, an acronym ("PM") or a contraction
/// ("I've", "J'avoue") says nothing about a name.
fn written_as_name(span: &[&str], opens_sentence: bool) -> bool {
    let [word] = span else {
        return false;
    };
    // Trimming, not slicing between the punctuation edges: a word of
    // punctuation alone ("?", "«") reports itself as both edges.
    let body = word.trim_matches(|c: char| !c.is_alphanumeric());
    let letters: Vec<char> = body.chars().filter(|c| c.is_alphabetic()).collect();
    let acronym = letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase());
    !opens_sentence
        && !acronym
        && body.chars().all(char::is_alphanumeric)
        && body.starts_with(char::is_uppercase)
}

/// Whether the word after `before` opens a sentence. A closing mark written
/// apart, as in « on a gagné ! » Allez, is looked through to the word it
/// closes, and a title's full stop ("Dr.", "M.") ends no sentence.
fn opens_sentence(before: &[&str]) -> bool {
    for word in before.iter().rev() {
        if extract_punctuation(word)
            .1
            .contains(['.', '!', '?', ':', '…'])
        {
            return !is_title(word);
        }
        if word.chars().any(char::is_alphanumeric) {
            return false;
        }
    }
    true
}

/// The closest custom-word key for a span whose edge words both belong to
/// the match.
fn match_span<'k>(
    span: &[&str],
    opens_sentence: bool,
    custom_word_match_keys: &'k [CustomWordMatchKey],
    threshold: f64,
) -> Option<(&'k CustomWordMatchKey, f64)> {
    let candidate = build_ngram(span);
    let written_as_name = written_as_name(span, opens_sentence);
    let (key, score) = find_best_match(
        &candidate,
        custom_word_match_keys,
        threshold,
        written_as_name,
    )?;
    // An everyday word is what the user said, unless it is the custom word
    // itself, accents included: "an" never becomes "Anne", "mari" never
    // becomes "Marie", "aime" never becomes "Aimé". A first name is not one:
    // "Lucas" becomes the user's "Lukas".
    let spelling: String = span.iter().map(|word| word_key(word)).collect();
    if span.len() == 1 && spelling != key.spelling && is_everyday_word(&spelling) {
        return None;
    }
    // In French an accent makes a different word: "marié" is not "Marie",
    // "câline" is not "Céline". An accented span spelled otherwise than the
    // custom word is taken for it only when capitalised as a name and the
    // accents are the only difference, as in "merci Clément" for the user's
    // "Clement": "Léa" never becomes "Léo".
    if spelling != candidate
        && spelling != key.spelling
        && !(written_as_name && candidate == key.key)
    {
        return None;
    }
    // An edge word that matches no part of the custom word, or does not
    // bring the span closer to it, is an ordinary word the span would
    // swallow: "my" in "my I phone" for iPhone, "is" in "CHARGE B is" for
    // ChargeBee, "app" in "chat app" for ChatGPT.
    let len = span.len();
    if len > 1 {
        let (first, last) = (build_match_key(span[0]), build_match_key(span[len - 1]));
        let without_first = match_score(&build_ngram(&span[1..]), &key.key, false);
        let without_last = match_score(&build_ngram(&span[..len - 1]), &key.key, false);
        if best_part_score(&first, &key.key) >= threshold
            || best_part_score(&last, &key.key) >= threshold
            || without_first <= score
            || without_last <= score
        {
            return None;
        }
    }
    Some((key, score))
}

const ENDINGS: [&str; 4] = ["'s", "’s", "es", "s"];

/// Each ending `word` carries, with the word cut before it. The cut word
/// keeps its trailing punctuation and must still end in a letter or digit.
fn endings(word: &str) -> impl Iterator<Item = (String, &str)> {
    let (_, punctuation) = extract_punctuation(word);
    let body = &word[..word.len() - punctuation.len()];
    ENDINGS.into_iter().filter_map(move |ending| {
        let cut = body.len().checked_sub(ending.len())?;
        let (stem, found) = (body.get(..cut)?, body.get(cut..)?);
        (found.eq_ignore_ascii_case(ending) && stem.ends_with(char::is_alphanumeric))
            .then(|| (format!("{stem}{punctuation}"), found))
    })
}

/// French elisions, written against the next word: "d'Anthropic".
const ELISIONS: [&str; 12] = [
    "c", "d", "j", "l", "m", "n", "s", "t", "qu", "jusqu", "lorsqu", "puisqu",
];

/// A French elision written against a word, with any punctuation before
/// it, and the rest of the word: "«l'" and "iPhone" for "«l'iPhone".
/// "O'Brien" has none: "o" is not an elision.
fn split_elision(word: &str) -> Option<(&str, &str)> {
    let (at, apostrophe) = word.char_indices().find(|(_, c)| matches!(c, '\'' | '’'))?;
    let (elision, rest) = word.split_at(at + apostrophe.len_utf8());
    let head = word[..at].trim_start_matches(|c: char| !c.is_alphanumeric());
    (ELISIONS.contains(&head.to_lowercase().as_str()) && rest.starts_with(char::is_alphanumeric))
        .then_some((elision, rest))
}

/// Every span of up to [`MAX_SPAN_WORDS`] words that matches a custom word and
/// whose edge words both belong to the match. A French elision and a
/// possessive ending are set aside before matching whenever that brings the
/// span closer. A plural
/// ending only when the whole span already matched, so "dans" never
/// becomes a plural of the name "Dan".
fn candidate_spans<'a>(
    words: &[&'a str],
    custom_words: &'a [String],
    custom_word_match_keys: &[CustomWordMatchKey],
    threshold: f64,
) -> Vec<SpanMatch<'a>> {
    let mut spans = Vec::new();
    for start in 0..words.len() {
        for len in 1..=MAX_SPAN_WORDS.min(words.len() - start) {
            let span = &words[start..start + len];
            // Do not consume across a punctuation boundary. In
            // "Charge B, che", the comma closes the candidate at "B,".
            if span[..len - 1]
                .iter()
                .any(|word| !extract_punctuation(word).1.is_empty())
            {
                continue;
            }
            let opens_sentence = opens_sentence(&words[..start]);
            let mut best = None;
            let elided = split_elision(span[0]).map(|(elision, rest)| {
                let mut span = span.to_vec();
                span[0] = rest;
                (elision, span)
            });
            // An elision before an everyday word is never part of a name:
            // "l'aura" is not "Laura", "d'une" is not "Dune".
            let unsplit = elided
                .as_ref()
                .is_none_or(|(_, rest)| !is_everyday_word(&word_key(rest[0])))
                .then(|| ("", span.to_vec()));
            for (elision, span) in unsplit.into_iter().chain(elided) {
                let whole = match_span(&span, opens_sentence, custom_word_match_keys, threshold);
                if let Some((key, score)) = whole {
                    if best.is_none_or(|(_, best_score, _, _)| score < best_score) {
                        best = Some((key, score, elision, ""));
                    }
                }
                for (stem, ending) in endings(span[len - 1]) {
                    if whole.is_none() && !ending.starts_with(['\'', '’']) {
                        continue;
                    }
                    let mut stemmed = span.clone();
                    stemmed[len - 1] = &stem;
                    if let Some((key, score)) =
                        match_span(&stemmed, opens_sentence, custom_word_match_keys, threshold)
                    {
                        if best.is_none_or(|(_, best_score, _, _)| score < best_score) {
                            best = Some((key, score, elision, ending));
                        }
                    }
                }
            }
            if let Some((key, score, elision, ending)) = best {
                spans.push(SpanMatch {
                    start,
                    len,
                    replacement: &custom_words[key.word_index],
                    elision,
                    ending,
                    score,
                });
            }
        }
    }
    spans
}

/// Applies custom word corrections to transcribed text using fuzzy matching
///
/// This function corrects words in the input text by finding the best matches
/// from a list of custom words using a combination of:
/// - Levenshtein distance for string similarity
/// - Soundex phonetic matching for pronunciation similarity
/// - N-gram matching for multi-word speech artifacts (e.g., "Charge B" -> "ChargeBee")
///
/// # Arguments
/// * `text` - The input text to correct
/// * `custom_words` - List of custom words to match against
/// * `threshold` - Maximum similarity score to accept (0.0 = exact match, 1.0 = any match)
///
/// # Returns
/// The corrected text with custom words applied
pub fn apply_custom_words(text: &str, custom_words: &[String], threshold: f64) -> String {
    apply_custom_words_counted(text, custom_words, threshold).0
}

/// [`apply_custom_words`] that also returns how many replacements changed
/// the text, for the Insights "dictionary fixes" count.
pub fn apply_custom_words_counted(
    text: &str,
    custom_words: &[String],
    threshold: f64,
) -> (String, u32) {
    if custom_words.is_empty() {
        return (text.to_string(), 0);
    }

    // Pre-compute normalized comparison keys to avoid repeated allocations.
    let custom_word_match_keys: Vec<CustomWordMatchKey> = custom_words
        .iter()
        .enumerate()
        .flat_map(|(index, word)| build_custom_word_match_keys(word, index))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();

    // Take the closest spans first, so an exact match is never pre-empted by
    // an overlapping fuzzy one that starts earlier. On a tie, the shorter
    // span wins, then the later one.
    let mut spans = candidate_spans(&words, custom_words, &custom_word_match_keys, threshold);
    spans.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then(a.len.cmp(&b.len))
            .then(b.start.cmp(&a.start))
    });
    let mut chosen: Vec<Option<&SpanMatch>> = vec![None; words.len()];
    let mut taken = vec![false; words.len()];
    for span in &spans {
        let range = span.start..span.start + span.len;
        if taken[range.clone()].contains(&true) {
            continue;
        }
        taken[range].fill(true);
        chosen[span.start] = Some(span);
    }

    let mut result = Vec::new();
    let mut replacements = 0;
    let mut i = 0;
    while i < words.len() {
        let Some(span) = chosen[i] else {
            result.push(words[i].to_string());
            i += 1;
            continue;
        };
        let span_words = &words[i..i + span.len];
        let original = span_words.join(" ");
        // Keep the punctuation around the first and last words of the span,
        // and the elision, which already holds the punctuation before it.
        let prefix = match span.elision {
            "" => extract_punctuation(span_words[0]).0,
            elision => elision,
        };
        let (_, suffix) = extract_punctuation(span_words[span.len - 1]);
        let corrected = preserve_case_pattern(&original[span.elision.len()..], span.replacement);
        let replaced = format!("{prefix}{corrected}{}{suffix}", span.ending);
        if replaced != original {
            replacements += 1;
        }
        result.push(replaced);
        i += span.len;
    }

    (result.join(" "), replacements)
}

/// Carries the case of the matched span over to the replacement. A shouted
/// span ("CHARGE B") stays shouted; a capitalised span capitalises a plain
/// replacement. A replacement with its own inner capitals ("iPhone") keeps
/// its spelling, and a lone capital letter ("I phone") is not shouting.
fn preserve_case_pattern(original: &str, replacement: &str) -> String {
    let letters: Vec<char> = original.chars().filter(|c| c.is_alphabetic()).collect();
    let has_inner_capital = replacement.chars().skip(1).any(char::is_uppercase);
    if letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if has_inner_capital {
        replacement.to_string()
    } else if letters.first().is_some_and(|c| c.is_uppercase()) {
        let mut chars: Vec<char> = replacement.chars().collect();
        if let Some(first_char) = chars.get_mut(0) {
            *first_char = first_char.to_uppercase().next().unwrap_or(*first_char);
        }
        chars.into_iter().collect()
    } else {
        replacement.to_string()
    }
}

/// Extracts punctuation prefix and suffix from a word
fn extract_punctuation(word: &str) -> (&str, &str) {
    // String slices use byte offsets. Derive both boundaries from char_indices
    // so multibyte punctuation such as `。` and `「」` can never be split.
    let prefix_end = word
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(word.len());
    let suffix_start = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, c)| index + c.len_utf8())
        .unwrap_or(0);

    let prefix = if prefix_end > 0 {
        &word[..prefix_end]
    } else {
        ""
    };

    let suffix = if suffix_start < word.len() {
        &word[suffix_start..]
    } else {
        ""
    };

    (prefix, suffix)
}

static MULTI_SPACE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Collapses repeated words (3+ repetitions) to a single instance.
/// E.g., "wh wh wh wh" -> "wh", "I I I I" -> "I"
fn collapse_stutters(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let word = words[i];
        let word_lower = word.to_lowercase();

        if word_lower.chars().all(|c| c.is_alphabetic()) {
            let mut count = 1;
            while i + count < words.len() && words[i + count].to_lowercase() == word_lower {
                count += 1;
            }

            if count >= 3 {
                result.push(word);
                i += count;
            } else {
                result.push(word);
                i += 1;
            }
        } else {
            result.push(word);
            i += 1;
        }
    }

    result.join(" ")
}

/// Collapses stutters and repeated whitespace. Fillers and spoken formatting
/// are `text_passes`' job.
pub fn normalize_transcription_output(text: &str) -> String {
    let mut normalized = collapse_stutters(text);

    normalized = MULTI_SPACE_PATTERN
        .replace_all(&normalized, " ")
        .to_string();

    normalized.trim().to_string()
}

#[cfg(test)]
mod tests;
