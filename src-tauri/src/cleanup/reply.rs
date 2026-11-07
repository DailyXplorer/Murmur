//! Whether a model reply is a cleanup of the dictation or something else: an
//! answer, a summary, a translation, commentary or a preamble. The two texts
//! are aligned character by character, so the check is the same for scripts
//! that put spaces between words and for Chinese, Japanese or Thai.

use super::PromptContext;
use crate::settings::DictationStyle;

/// Stands for any run of spaces and punctuation. It weighs nothing, but a
/// letter next to it in both texts is anchored at a word edge.
const GAP: char = ' ';
/// Stands for an apostrophe. It weighs nothing and is dropped before the
/// texts are aligned ("don't" matches "dont"), but it marks a word edge
/// while terms are blanked: "d'OpenAI", "dell'iPhone", "OpenAI's".
const APOSTROPHE: char = '\'';
/// A cleanup may add this many weight units anywhere ("too", "know",
/// digits), plus one unit in `NEW_SHARE`.
const NEW_SLACK: u32 = 2;
const NEW_SHARE: u32 = 10;
/// Preambles and sign-offs sit at the ends of a reply, where a long cleanup
/// would otherwise hide them in its overall allowance. The first and the last
/// `EDGE_UNITS` of a reply may add at most `EDGE_SLACK` units each.
const EDGE_UNITS: u32 = 16;
const EDGE_SLACK: u32 = 5;
/// The spoken forms the formal style writes in full, as (written, spoken),
/// each before any shorter form it contains. In that style both texts read
/// every written form as the spoken one before they are aligned, so these
/// rewordings cost nothing and the budget stays that of the other styles.
const REGISTER: [(&str, &str); 15] = [
    ("il faut que nous", "faut qu'on"),
    ("il faut que", "faut que"),
    ("que nous", "qu'on"),
    ("nous pouvons", "on peut"),
    ("cela me convient", "ça marche"),
    ("me convient", "ça marche"),
    ("convient", "ça marche"),
    ("fonctionne", "marche"),
    ("tu es", "t'es"),
    ("tu as", "t'as"),
    ("as-tu", "t'as"),
    ("disponible", "dispo"),
    ("going to", "gonna"),
    ("i will", "i'll"),
    ("hello", "hey"),
];
/// A reply under this many tenths of the dictation drops what was said: a
/// summary, or an answer ("No.", "4."). Fillers are not counted in the
/// dictation. Neither is a restart cue trusted: "no wait" or "scratch that"
/// can be meant, and an answer to the question before it must not pass. So
/// a cleanup that drops a long false start can fall under the floor; the
/// raw transcript is pasted then, and nothing is lost.
const MIN_KEPT_TENTHS: u32 = 3;
/// Whole-word hesitations the prompt asks to remove, in its languages.
const FILLERS: [&str; 17] = [
    "um", "umm", "uh", "uhm", "hmm", "hm", "mm", "er", "erm", "euh", "heu", "äh", "ähm", "öh",
    "ehm", "eh", "ah",
];

/// Rejects a reply that is not a plausible cleanup of `dictation`: empty,
/// wrapped in our tags, far longer or shorter than the input, or adding
/// more than a few characters to what the speaker said.
pub fn accept_output(dictation: &str, output: &str, context: &PromptContext) -> Option<String> {
    let cleaned = output.trim();
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_lowercase();
    if ["<dictation", "</dictation", "<field", "</field", "<cursor"]
        .iter()
        .any(|tag| lower.contains(tag))
    {
        return None;
    }
    if cleaned.chars().count() > max_output_chars(dictation.chars().count()) {
        return None;
    }
    is_an_edit(dictation, cleaned, context).then(|| cleaned.to_string())
}

/// Lists and line breaks add a little markup, never whole sentences.
pub fn max_output_chars(input_chars: usize) -> usize {
    input_chars + input_chars / 2 + 40
}

fn is_an_edit(dictation: &str, reply: &str, context: &PromptContext) -> bool {
    let register = |text| match context.style {
        DictationStyle::Formal => in_spoken_register(text),
        DictationStyle::Normal | DictationStyle::Casual => text,
    };
    let spoken = register(without_apostrophes(units(dictation)));
    let reply = units(reply);
    let reply_weight = weight(&reply);
    if reply_weight * 10 < weight(&units(&without_fillers(dictation))) * MIN_KEPT_TENTHS {
        return false;
    }
    let terms: Vec<Vec<char>> = context
        .vocabulary
        .iter()
        .chain(&context.names)
        .map(|term| units(term))
        .collect();
    let reply = register(without_apostrophes(without_terms(reply, &terms)));

    let head = split_at_weight(&reply, EDGE_UNITS);
    let tail = split_at_weight(&reply, weight(&reply).saturating_sub(EDGE_UNITS));
    let (matched, head_matched, tail_matched) = aligned(&reply, &spoken, head, tail);

    let added = weight(&reply) - matched;
    let head_added = weight(&reply[..head]) - head_matched;
    let tail_added = weight(&reply[tail..]) - tail_matched;
    added <= NEW_SLACK + reply_weight / NEW_SHARE
        && head_added <= EDGE_SLACK
        && tail_added <= EDGE_SLACK
}

/// `units` with each written form of `REGISTER` read as its spoken one.
fn in_spoken_register(mut text: Vec<char>) -> Vec<char> {
    for (written, spoken) in REGISTER {
        let written = without_apostrophes(units(written));
        let spoken = without_apostrophes(units(spoken));
        let mut out = Vec::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            // Both forms are framed by gaps; the closing one is kept for
            // the next word, which may start another match.
            if text[i..].starts_with(&written) {
                out.extend_from_slice(&spoken[..spoken.len() - 1]);
                i += written.len() - 1;
            } else {
                out.push(text[i]);
                i += 1;
            }
        }
        text = out;
    }
    text
}

/// The dictation's words without fillers.
fn without_fillers(dictation: &str) -> String {
    dictation
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty() && !FILLERS.contains(&word.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase letters and digits with every run of other characters folded
/// into one `GAP`, framed by gaps. Each apostrophe becomes an `APOSTROPHE`.
fn units(text: &str) -> Vec<char> {
    let mut units = vec![GAP];
    for c in text.chars().flat_map(char::to_lowercase) {
        if matches!(c, '\'' | '’') {
            units.push(APOSTROPHE);
        } else if c.is_alphanumeric() || is_thai(c) || ('\u{300}'..='\u{36f}').contains(&c) {
            units.push(c);
        } else if units.last() != Some(&GAP) {
            units.push(GAP);
        }
    }
    if units.last() != Some(&GAP) {
        units.push(GAP);
    }
    units
}

/// Apostrophes vanish from the aligned texts ("don't" -> "dont").
fn without_apostrophes(mut units: Vec<char>) -> Vec<char> {
    units.retain(|&c| c != APOSTROPHE);
    units
}

/// Thai vowel signs and tone marks are not all alphabetic, but they are
/// part of the word.
fn is_thai(c: char) -> bool {
    ('\u{e01}'..='\u{e4e}').contains(&c)
}

/// A Han character carries about a short word, a kana or Hangul syllable
/// about two letters.
fn unit_weight(c: char) -> u32 {
    match c {
        GAP | APOSTROPHE => 0,
        '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' => 3,
        '\u{20000}'..='\u{3134f}' => 3,
        '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}' => 2,
        _ => 1,
    }
}

fn weight(units: &[char]) -> u32 {
    units.iter().map(|&c| unit_weight(c)).sum()
}

/// Turns the vocabulary and window names found in the reply into gaps: the
/// reply may spell them even though the speaker said something else. A term
/// written with spaces between words counts only as whole words, or before
/// a plural or possessive "s": "ai" is not blanked out of "said", but
/// "figma" is out of "Figmas". An apostrophe is a word edge, so "openai" is
/// blanked out of "d'OpenAI" and "OpenAI's".
fn without_terms(mut reply: Vec<char>, terms: &[Vec<char>]) -> Vec<char> {
    let spaced = |c: char| unit_weight(c) == 1 && !is_thai(c);
    for term in terms {
        let term = term.get(1..term.len() - 1).unwrap_or_default();
        if weight(term) < 2 || term.len() > reply.len() {
            continue;
        }
        let mut start = 0;
        while start + term.len() <= reply.len() {
            let end = start + term.len();
            let edge = |inside: char, index: Option<usize>| {
                index
                    .and_then(|i| reply.get(i))
                    .is_none_or(|&outside| outside == GAP || !spaced(inside) || !spaced(outside))
            };
            let ending_s = reply.get(end) == Some(&'s') && edge('s', Some(end + 1));
            let whole = edge(term[0], start.checked_sub(1))
                && (edge(term[term.len() - 1], Some(end)) || ending_s);
            if whole && reply[start..end] == *term {
                reply[start..start + term.len()].fill(GAP);
                start += term.len();
            } else {
                start += 1;
            }
        }
    }
    reply
}

/// The first index where the weight of `units[..index]` reaches `target`.
fn split_at_weight(units: &[char], target: u32) -> usize {
    let mut total = 0;
    for (index, &c) in units.iter().enumerate() {
        if total >= target {
            return index;
        }
        total += unit_weight(c);
    }
    units.len()
}

/// Whether reply unit `i` may align with dictation unit `j`. A lone Latin
/// or Thai letter matches by chance, so it needs an equal neighbour in the
/// same word on one side, next to it or one letter further ("their" ->
/// "they're"), or must be a one-letter word in both texts. Han, kana and
/// Hangul carry enough on their own.
fn anchored(reply: &[char], spoken: &[char], i: usize, j: usize) -> bool {
    let c = reply[i];
    if c == GAP || c != spoken[j] {
        return false;
    }
    if unit_weight(c) >= 2 {
        return true;
    }
    let at = |units: &[char], index: usize, step: isize| {
        index
            .checked_add_signed(step)
            .and_then(|index| units.get(index))
            .copied()
            .unwrap_or(GAP)
    };
    let joined = |step: isize| {
        let near = (at(reply, i, step), at(spoken, j, step));
        let far = (at(reply, i, 2 * step), at(spoken, j, 2 * step));
        near.0 != GAP && near.1 != GAP && (near.0 == near.1 || (far.0 == far.1 && far.0 != GAP))
    };
    let lone =
        |units: &[char], index: usize| at(units, index, -1) == GAP && at(units, index, 1) == GAP;
    joined(-1) || joined(1) || (lone(reply, i) && lone(spoken, j))
}

/// Weighted longest common subsequence of `reply[..stop]` against
/// `spoken[..j]`, for every `j` and each stop, in linear memory.
fn prefix_scores(reply: &[char], spoken: &[char], stops: [usize; 2]) -> [Vec<u32>; 2] {
    let mut row = vec![0; spoken.len() + 1];
    let mut next = row.clone();
    let mut scores = [Vec::new(), Vec::new()];
    for i in 0..=stops[0].max(stops[1]) {
        for (stop, score) in stops.iter().zip(&mut scores) {
            if *stop == i {
                score.clone_from(&row);
            }
        }
        if i == reply.len() {
            break;
        }
        for j in 0..spoken.len() {
            let diagonal = if anchored(reply, spoken, i, j) {
                row[j] + unit_weight(reply[i])
            } else {
                0
            };
            next[j + 1] = row[j + 1].max(next[j]).max(diagonal);
        }
        std::mem::swap(&mut row, &mut next);
    }
    scores
}

/// Matched weight of a best alignment of the whole reply, and how much of
/// it the first `head` units and the units from `tail` on can each claim
/// in some best alignment.
fn aligned(reply: &[char], spoken: &[char], head: usize, tail: usize) -> (u32, u32, u32) {
    let [before_head, before_tail] = prefix_scores(reply, spoken, [head, tail]);
    let reversed = |units: &[char]| units.iter().rev().copied().collect::<Vec<_>>();
    let ends = [reply.len() - head, reply.len() - tail];
    let [after_head, after_tail] = prefix_scores(&reversed(reply), &reversed(spoken), ends);
    // The reversed scores are indexed by how much of the dictation's end
    // they cover; `j` is where the rest of the dictation starts.
    let m = spoken.len();
    let total = (0..=m)
        .map(|j| before_head[j] + after_head[m - j])
        .max()
        .unwrap_or(0);
    let claim = |before: &[u32], after: &[u32], part: fn(u32, u32) -> u32| {
        (0..=m)
            .filter(|&j| before[j] + after[m - j] == total)
            .map(|j| part(before[j], after[m - j]))
            .max()
            .unwrap_or(0)
    };
    let head_matched = claim(&before_head, &after_head, |before, _| before);
    let tail_matched = claim(&before_tail, &after_tail, |_, after| after);
    (total, head_matched, tail_matched)
}

#[cfg(test)]
mod tests;
