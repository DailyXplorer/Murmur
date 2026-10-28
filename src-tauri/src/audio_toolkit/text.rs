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

/// Collapses stutters and repeated whitespace. Fillers and formatting are
/// the AI cleanup's job.
pub fn normalize_transcription_output(text: &str) -> String {
    let mut normalized = collapse_stutters(text);

    normalized = MULTI_SPACE_PATTERN
        .replace_all(&normalized, " ")
        .to_string();

    normalized.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default custom-word threshold (`settings.rs`).
    const THRESHOLD: f64 = 0.18;

    const PRODUCTS: &[&str] = &["iPhone", "ChatGPT", "OpenAI", "ChargeBee"];
    const ENDING_WORDS: &[&str] = &["ChatGPT", "iPhone", "Figma", "Dan"];
    const ELISION_WORDS: &[&str] = &[
        "iPhone",
        "Anthropic",
        "Excel",
        "D'Angelo",
        "Apple",
        "Emile",
        "Laura",
        "Dune",
    ];

    /// Every dictated sentence the custom-word pass has been checked on:
    /// custom words, what Codex wrote, what is pasted. Add a row for each
    /// new case; never edit an old one to make a change pass.
    const CORPUS: &[(&[&str], &str, &str)] = &[
        (&[], "hello world", "hello world"),
        (&["Hello", "World"], "hello world", "Hello World"),
        (&["hello", "Murmur"], "helo from Murmur", "helo from Murmur"),
        // A lone capital letter is not shouting.
        (&["iPhone", "Xcode"], "I phone died", "iPhone died"),
        (&["iPhone", "Xcode"], "open X code now", "open Xcode now"),
        (
            &["iPhone", "Xcode"],
            "Hi. X code crashed.",
            "Hi. Xcode crashed.",
        ),
        // A fuzzy span starting one word early must not beat the exact span
        // after it.
        (PRODUCTS, "my I phone died", "my iPhone died"),
        (PRODUCTS, "the chat GPT app", "the ChatGPT app"),
        (PRODUCTS, "open AI is", "OpenAI is"),
        (PRODUCTS, "charge B is great", "ChargeBee is great"),
        // A trailing ordinary word adds nothing to the match.
        (
            &["ChargeBee"],
            "we use charge it daily",
            "we use charge it daily",
        ),
        (&["ChatGPT"], "the chat app works", "the chat app works"),
        // Adjacent custom words each keep their own span.
        (PRODUCTS, "open AI chat GPT", "OpenAI ChatGPT"),
        (
            PRODUCTS,
            "my I phone, chat GPT and charge B.",
            "my iPhone, ChatGPT and ChargeBee.",
        ),
        (PRODUCTS, "mon I phone est mort", "mon iPhone est mort"),
        (
            PRODUCTS,
            "j'utilise chat GPT tous les jours",
            "j'utilise ChatGPT tous les jours",
        ),
        (
            PRODUCTS,
            "on a signé avec charge B hier",
            "on a signé avec ChargeBee hier",
        ),
        (
            PRODUCTS,
            "le modèle open AI est rapide",
            "le modèle OpenAI est rapide",
        ),
        (PRODUCTS, "we met at noon", "we met at noon"),
        (PRODUCTS, "je mange une pomme", "je mange une pomme"),
        (PRODUCTS, "my iPhone died", "my iPhone died"),
        // Multi-word spans.
        (
            &["ChargeBee"],
            "il cui nome è Charge B, che permette",
            "il cui nome è ChargeBee, che permette",
        ),
        (
            &["ChatGPT"],
            "use Chat G P T for this",
            "use ChatGPT for this",
        ),
        (&["OpenAI", "GPT"], "Open AI GPT model", "OpenAI GPT model"),
        (&["ChargeBee"], "CHARGE B is great", "CHARGEBEE is great"),
        (&["MacBook Pro"], "using Mac Book Pro", "using MacBook Pro"),
        (&["GPT-4"], "use GPT4 for this", "use GPT-4 for this"),
        (
            &["R&D"],
            "send it to RD for review",
            "send it to R&D for review",
        ),
        (
            &["R&D"],
            "send it to R and D for review",
            "send it to R&D for review",
        ),
        (
            &["R&D"],
            "send it to R&D for review",
            "send it to R&D for review",
        ),
        (&["Murmur"], "「Murmurr。」", "「Murmur。」"),
        // Possessive and plural endings stay on the custom word.
        (ENDING_WORDS, "ChatGPT's answer", "ChatGPT's answer"),
        (ENDING_WORDS, "ChatGPT’s answer", "ChatGPT’s answer"),
        (ENDING_WORDS, "both iPhones died", "both iPhones died"),
        (ENDING_WORDS, "two Figmas.", "two Figmas."),
        (ENDING_WORDS, "ask chat gpt's view", "ask ChatGPT's view"),
        (ENDING_WORDS, "my I phones", "my iPhones"),
        (
            ENDING_WORDS,
            "je vais dans le bureau",
            "je vais dans le bureau",
        ),
        // A short name never rewrites an everyday word.
        (&["Anne"], "I ate an apple", "I ate an apple"),
        (&["Anne"], "il y a un an", "il y a un an"),
        (&["Marc"], "ça marche bien", "ça marche bien"),
        (&["Marie"], "mon mari arrive", "mon mari arrive"),
        (&["Jean"], "j'en ai marre", "j'en ai marre"),
        (&["Paul"], "un pull rouge", "un pull rouge"),
        // The name itself, in any case, still takes the user's spelling.
        (&["Anne"], "ask anne first", "ask Anne first"),
        (&["Marc"], "MARC est là", "MARC est là"),
        // A short name with one letter wrong is corrected mid-sentence.
        (&["Zach"], "ask Zack first", "ask Zach first"),
        (&["Luka"], "je vois Luca demain", "je vois Luka demain"),
        (&["Sarah"], "call Sara today", "call Sarah today"),
        (&["Zach"], "ask Zack's mother", "ask Zach's mother"),
        (
            &["Zach"],
            "j'ai dit «Zack» deux fois",
            "j'ai dit «Zach» deux fois",
        ),
        // A title's full stop ends no sentence.
        (&["Zach"], "ask Dr. Zack first", "ask Dr. Zach first"),
        (&["Luka"], "j'ai vu M. Luca hier", "j'ai vu M. Luka hier"),
        (&["Zach"], "ask Ms. Zack first", "ask Ms. Zach first"),
        (&["Zach"], "ask Prof. Zack first", "ask Prof. Zach first"),
        (
            &["Luka"],
            "j'ai vu le Pr. Luca hier",
            "j'ai vu le Pr. Luka hier",
        ),
        // An acronym or a unit before a full stop is no title.
        (
            &["Nina"],
            "I opened a PR. Nine people came.",
            "I opened a PR. Nine people came.",
        ),
        (
            &["Nina"],
            "We ran 3 m. Nine people came.",
            "We ran 3 m. Nine people came.",
        ),
        // A short everyday word that sounds like a name is kept.
        (&["Lou"], "je l'ai lu hier", "je l'ai lu hier"),
        (&["Mona"], "c'est mon idée", "c'est mon idée"),
        (&["Mona"], "Mon idée est simple", "Mon idée est simple"),
        (&["Moon"], "mon téléphone", "mon téléphone"),
        (&["May"], "le 3 mai", "le 3 mai"),
        // Not an everyday word on the list: only the length keeps it.
        (&["Liz"], "je lis un livre", "je lis un livre"),
        // A capital that does not mark a name: a proper noun, a contraction,
        // an acronym, or the first word of a sentence.
        (&["Max"], "my Mac is slow", "my Mac is slow"),
        (&["Max"], "Mon Mac plante", "Mon Mac plante"),
        (&["Ivy"], "I've sent it", "I've sent it"),
        (&["Ilya"], "I'll call you", "I'll call you"),
        (&["Leon"], "Je vais à Lyon", "Je vais à Lyon"),
        (&["Mona"], "I told Mom", "I told Mom"),
        (&["Nina"], "Nine people came.", "Nine people came."),
        (
            &["Nina"],
            "Done. Nine people came.",
            "Done. Nine people came.",
        ),
        (&["Sarah"], "Sara, can you call me", "Sara, can you call me"),
        (&["Sarah"], "Présents : Sara", "Présents : Sara"),
        // A closing mark written apart, after the sentence end.
        (
            &["Alex"],
            "Il a crié « on a gagné ! » Allez, on rentre.",
            "Il a crié « on a gagné ! » Allez, on rentre.",
        ),
        (
            &["Alex"],
            "Il a crié \" on a gagné ! \" Allez, on rentre.",
            "Il a crié \" on a gagné ! \" Allez, on rentre.",
        ),
        (
            &["Alex"],
            "Il a crié (on a gagné ! ) Allez, on rentre.",
            "Il a crié (on a gagné ! ) Allez, on rentre.",
        ),
        (&["Pam"], "the PM said", "the PM said"),
        (&["Jess"], "le code JS", "le code JS"),
        (&["Avi"], "the API returns", "the API returns"),
        (
            &["Java"],
            "J'avoue, c'est pratique",
            "J'avoue, c'est pratique",
        ),
        (&["Benny"], "Ben, je sais pas", "Ben, je sais pas"),
        (&["Benny"], "ouais Ben je sais pas", "ouais Ben je sais pas"),
        // A first name that is also an everyday word is kept.
        (&["Clara"], "une réponse claire", "une réponse claire"),
        (&["Ross"], "un pull rose", "un pull rose"),
        (&["Rosa"], "la robe rose", "la robe rose"),
        (&["Jeanne"], "un jean bleu", "un jean bleu"),
        (&["Pierce"], "une pierre", "une pierre"),
        (
            &["Simon"],
            "Sinon, on verra demain",
            "Sinon, on verra demain",
        ),
        (
            &["Simon"],
            "appelle-moi, sinon je t'écris",
            "appelle-moi, sinon je t'écris",
        ),
        // A custom spelling overrides a common first name.
        (&["Lukas"], "Lucas a appelé", "Lukas a appelé"),
        (&["Nicholas"], "merci Nicolas", "merci Nicholas"),
        (&["Sofie"], "Sophie arrive", "Sofie arrive"),
        (&["Tomas"], "Thomas arrive", "Tomas arrive"),
        (&["Philipe"], "Philippe arrive", "Philipe arrive"),
        (&["O'Brien"], "call o'brien today", "call O'Brien today"),
        // An accent the transcriber dropped or added is put back as the
        // user wrote it.
        (&["Zoé"], "appelle Zoe demain", "appelle Zoé demain"),
        (&["Léa"], "j'ai vu Lea hier", "j'ai vu Léa hier"),
        (&["Anaïs"], "merci Anais", "merci Anaïs"),
        (&["Loïc"], "dis à Loic de venir", "dis à Loïc de venir"),
        (&["Noémie"], "Noemie arrive", "Noémie arrive"),
        (&["Clement"], "merci Clément", "merci Clement"),
        (&["Zoé"], "c'est Zoé.", "c'est Zoé."),
        // An accented everyday word is still what the user said.
        (&["Zoé"], "c'était déjà fait", "c'était déjà fait"),
        (&["Annie"], "une bonne année", "une bonne année"),
        // An everyday word is not an accented name it folds to.
        (&["Aimé"], "je t'aime", "je t'aime"),
        (&["Aimé"], "il aime le chocolat", "il aime le chocolat"),
        (&["Côme"], "come here", "come here"),
        (&["Côme"], "Come on!", "Come on!"),
        (&["Côme"], "dis à côme de venir", "dis à Côme de venir"),
        // An accented name is not another name that differs by a letter.
        (&["Léo"], "j'ai vu Léa hier", "j'ai vu Léa hier"),
        (&["Léa"], "j'ai vu Léo hier", "j'ai vu Léo hier"),
        (&["Théo"], "merci Théa", "merci Théa"),
        // An accent the custom word lacks makes a different word, unless
        // the transcriber capitalised it as a name.
        (
            &["Marie"],
            "je suis marié depuis deux ans",
            "je suis marié depuis deux ans",
        ),
        (&["Marie"], "elle est mariée", "elle est mariée"),
        (&["Marie"], "ils sont mariés", "ils sont mariés"),
        (&["Céline"], "elle est câline", "elle est câline"),
        (&["Clement"], "un hiver clément", "un hiver clément"),
        // A French elision stays in front of the custom word.
        (ELISION_WORDS, "j'adore l'iPhone", "j'adore l'iPhone"),
        (
            ELISION_WORDS,
            "le modèle d'Anthropic",
            "le modèle d'Anthropic",
        ),
        (
            ELISION_WORDS,
            "le modèle d’Anthropic.",
            "le modèle d’Anthropic.",
        ),
        (ELISION_WORDS, "un fichier d'excel", "un fichier d'Excel"),
        (ELISION_WORDS, "L'I phone est mort", "L'iPhone est mort"),
        (
            ELISION_WORDS,
            "la musique de D'Angelo",
            "la musique de D'Angelo",
        ),
        // An everyday word after the elision is what the user said.
        (
            ELISION_WORDS,
            "Bonjour, je m'appelle Marc",
            "Bonjour, je m'appelle Marc",
        ),
        (ELISION_WORDS, "il s'appelle Paul", "il s'appelle Paul"),
        (ELISION_WORDS, "on s'appelle demain", "on s'appelle demain"),
        (ELISION_WORDS, "je t'appelle", "je t'appelle"),
        (ELISION_WORDS, "il appelle demain", "il appelle demain"),
        (ELISION_WORDS, "envoie-moi l'email", "envoie-moi l'email"),
        (ELISION_WORDS, "un email", "un email"),
        (ELISION_WORDS, "il l'aura demain", "il l'aura demain"),
        (ELISION_WORDS, "elle l'aura vu", "elle l'aura vu"),
        (ELISION_WORDS, "la fin d'une époque", "la fin d'une époque"),
        (ELISION_WORDS, "demande à laura", "demande à Laura"),
        // A word made only of punctuation stays as written; words are
        // joined back with plain spaces.
        (&["Max"], "Tu viens ?", "Tu viens ?"),
        (&["Max"], "C'est ça !", "C'est ça !"),
        (&["Max"], "Note : demain", "Note : demain"),
        (&["Max"], "« Bonjour »", "« Bonjour »"),
        (&["Max"], "Tu viens\u{202f}?", "Tu viens ?"),
        (&["Max"], "R & D", "R & D"),
        (&["Max"], "this – that", "this – that"),
        (&["Max"], "wait …", "wait …"),
    ];

    #[test]
    fn custom_word_corpus() {
        let failures: Vec<String> = CORPUS
            .iter()
            .filter_map(|(words, input, expected)| {
                let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
                let (text, fixes) = apply_custom_words_counted(input, &words, THRESHOLD);
                let respaced = input.split_whitespace().collect::<Vec<_>>().join(" ");
                (text != *expected || (fixes > 0) != (text != respaced)).then(|| {
                    format!("{words:?} {input:?}: got {text:?} ({fixes} fixes), want {expected:?}")
                })
            })
            .collect();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn test_apply_custom_words_fuzzy_match() {
        let text = "helo wrold";
        let custom_words = vec!["hello".to_string(), "world".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn counted_custom_words_skip_matches_that_change_nothing() {
        let custom_words = vec!["hello".to_string(), "Murmur".to_string()];
        let (text, replacements) =
            apply_custom_words_counted("helo from Murmur", &custom_words, 0.5);
        assert_eq!((text.as_str(), replacements), ("hello from Murmur", 1));
        assert_eq!(
            apply_custom_words_counted("plain text", &[], 0.5),
            ("plain text".to_string(), 0)
        );
    }

    #[test]
    fn test_preserve_case_pattern() {
        assert_eq!(preserve_case_pattern("HELLO", "world"), "WORLD");
        assert_eq!(preserve_case_pattern("Hello", "world"), "World");
        assert_eq!(preserve_case_pattern("hello", "WORLD"), "WORLD");
    }

    #[test]
    fn test_extract_punctuation() {
        assert_eq!(extract_punctuation("hello"), ("", ""));
        assert_eq!(extract_punctuation("!hello?"), ("!", "?"));
        assert_eq!(extract_punctuation("...hello..."), ("...", "..."));
    }

    #[test]
    fn test_extract_punctuation_uses_unicode_boundaries() {
        assert_eq!(extract_punctuation("你好。"), ("", "。"));
        assert_eq!(extract_punctuation("「你好」"), ("「", "」"));
        assert_eq!(extract_punctuation("你好！"), ("", "！"));
    }

    #[test]
    fn test_filter_cleans_whitespace() {
        let text = "Hello    world   test";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "Hello world test");
    }

    #[test]
    fn test_filter_trims() {
        let text = "  Hello world  ";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_filter_preserves_valid_text() {
        let text = "This is a completely normal sentence.";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "This is a completely normal sentence.");
    }

    #[test]
    fn test_filter_stutter_collapse() {
        let text = "w wh wh wh wh wh wh wh wh wh why";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "w wh why");
    }

    #[test]
    fn test_filter_stutter_short_words() {
        let text = "I I I I think so so so so";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "I think so");
    }

    #[test]
    fn test_filter_stutter_longer_words() {
        let text = "Check data doc doc doc doc documentation.";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "Check data doc documentation.");
    }

    #[test]
    fn test_filter_stutter_mixed_case() {
        let text = "No NO no NO no";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "No");
    }

    #[test]
    fn test_filter_stutter_preserves_two_repetitions() {
        let text = "no no is fine";
        let result = normalize_transcription_output(text);
        assert_eq!(result, "no no is fine");
    }

    #[test]
    fn test_apply_custom_words_skips_cjk_fuzzy_matching() {
        let text = "你好。";
        let custom_words = vec!["你号".to_string()];
        let result = apply_custom_words(text, &custom_words, 1.0);
        assert_eq!(result, text);
    }
}
