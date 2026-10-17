//! Pairs the words of two transcripts of the same audio: Codex's, which is
//! shown, and the on-device recogniser's, which carries times. A weighted
//! Needleman–Wunsch where the on-device words may start and end anywhere for
//! free, since they cover a little more audio than the Codex chunk.

/// Larger inputs are not aligned; the caller falls back to estimated times.
pub const MAX_CELLS: usize = 16_000_000;
/// The cost of a Codex word the on-device recogniser did not hear.
const CODEX_GAP: f32 = 0.7;
/// The cost of a word heard on device that Codex did not write: fillers,
/// or a number Codex wrote in digits and the device heard as several words.
/// Cheap, so skipping a few of them in the middle of a chunk is not dearer
/// than skipping them for free at its edges.
const LOCAL_GAP: f32 = 0.3;

/// Pairing two words earns up to 1 when they are equal and costs up to 1
/// when they are unrelated. A match must earn, not just cost less: since the
/// on-device words around the chunk are free to skip, a pairing that only
/// cost less would let the alignment skip a true match at the edges.
fn pair_cost(a: &str, b: &str) -> f32 {
    1.0 - 2.0 * similarity(a, b)
}

/// `1 − edit distance / longer length`: 1 for equal words, 0 for unrelated.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let longer = a.len().max(b.len());
    if longer == 0 {
        return 1.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = diagonal + usize::from(ca != cb);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    1.0 - row[b.len()] as f32 / longer as f32
}

/// One step of an alignment: a Codex word, an on-device word, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pair(usize, usize),
    Codex(usize),
    Local(usize),
}

const FROM_PAIR: u8 = 0;
const FROM_CODEX: u8 = 1;
const FROM_LOCAL: u8 = 2;

/// Aligns `codex` against `local` (comparison keys). Every Codex word appears
/// exactly once, in order. `None` when the inputs are too large.
pub fn align(codex: &[&str], local: &[&str]) -> Option<Vec<Step>> {
    let (n, m) = (codex.len(), local.len());
    if (n + 1).saturating_mul(m + 1) > MAX_CELLS {
        return None;
    }
    let width = m + 1;
    let mut from = vec![FROM_LOCAL; (n + 1) * width];
    // Leading on-device words cost nothing.
    let mut previous = vec![0.0f32; width];
    let mut current = vec![0.0f32; width];
    for i in 1..=n {
        current[0] = i as f32 * CODEX_GAP;
        from[i * width] = FROM_CODEX;
        for j in 1..=m {
            let pair = previous[j - 1] + pair_cost(codex[i - 1], local[j - 1]);
            let skip_codex = previous[j] + CODEX_GAP;
            let skip_local = current[j - 1] + LOCAL_GAP;
            let (cost, step) = if pair <= skip_codex && pair <= skip_local {
                (pair, FROM_PAIR)
            } else if skip_codex <= skip_local {
                (skip_codex, FROM_CODEX)
            } else {
                (skip_local, FROM_LOCAL)
            };
            current[j] = cost;
            from[i * width + j] = step;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    // Trailing on-device words cost nothing either: end where it is cheapest.
    let mut j = (0..=m)
        .min_by(|a, b| previous[*a].total_cmp(&previous[*b]))
        .unwrap_or(0);
    let mut steps: Vec<Step> = (j..m).rev().map(Step::Local).collect();
    let mut i = n;
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
            steps.push(Step::Local(j));
            continue;
        }
        match from[i * width + j] {
            FROM_PAIR => {
                i -= 1;
                j -= 1;
                steps.push(Step::Pair(i, j));
            }
            FROM_CODEX => {
                i -= 1;
                steps.push(Step::Codex(i));
            }
            _ => {
                j -= 1;
                steps.push(Step::Local(j));
            }
        }
    }
    steps.reverse();
    Some(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<&str> {
        text.split(' ').filter(|w| !w.is_empty()).collect()
    }

    fn pairs(codex: &str, local: &str) -> Vec<(usize, usize)> {
        align(&words(codex), &words(local))
            .unwrap()
            .into_iter()
            .filter_map(|step| match step {
                Step::Pair(c, l) => Some((c, l)),
                _ => None,
            })
            .collect()
    }

    fn codex_order(steps: &[Step]) -> Vec<usize> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Pair(c, _) | Step::Codex(c) => Some(*c),
                Step::Local(_) => None,
            })
            .collect()
    }

    #[test]
    fn similarity_is_one_for_equal_words_and_falls_with_edits() {
        assert_eq!(similarity("plateforme", "plateforme"), 1.0);
        assert!((similarity("plateforme", "platforme") - 0.9).abs() < 1e-6);
        assert_eq!(similarity("abc", "xyz"), 0.0);
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("", "a"), 0.0);
    }

    #[test]
    fn identical_sequences_pair_word_for_word() {
        assert_eq!(
            pairs("on livre jeudi", "on livre jeudi"),
            [(0, 0), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn words_missing_on_either_side_are_skipped() {
        // Codex wrote a word the recogniser missed, and the other way round.
        assert_eq!(
            pairs("bon alors on livre jeudi", "alors on livre euh jeudi"),
            [(1, 0), (2, 1), (3, 2), (4, 4)]
        );
    }

    #[test]
    fn extra_words_around_the_chunk_are_free() {
        let steps = align(
            &words("budget valide"),
            &words("fin de phrase budget valide et la suite"),
        )
        .unwrap();
        assert_eq!(
            steps,
            [
                Step::Local(0),
                Step::Local(1),
                Step::Local(2),
                Step::Pair(0, 3),
                Step::Pair(1, 4),
                Step::Local(5),
                Step::Local(6),
                Step::Local(7),
            ]
        );
    }

    #[test]
    fn a_match_at_the_edge_is_not_skipped_for_free() {
        // Skipping "objectif deux" costs nothing, but the true match wins.
        assert_eq!(
            pairs(
                "objectif 2030 pour tous",
                "objectif deux mille trente pour tous"
            )[0],
            (0, 0)
        );
    }

    #[test]
    fn close_spellings_still_pair() {
        assert_eq!(
            pairs("la plateforme marche", "la platforme marche"),
            [(0, 0), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn empty_inputs_keep_every_codex_word() {
        assert_eq!(
            align(&words("un deux"), &[]).unwrap(),
            [Step::Codex(0), Step::Codex(1)]
        );
        assert_eq!(align(&[], &words("un")).unwrap(), [Step::Local(0)]);
        assert!(align(&[], &[]).unwrap().is_empty());
    }

    #[test]
    fn oversized_inputs_are_refused() {
        let many = vec!["mot"; 5_000];
        assert!(align(&many, &many).is_none());
    }

    #[test]
    fn every_codex_word_comes_out_once_in_order() {
        let mut state: u32 = 7;
        let mut next = move |bound: u32| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) % bound
        };
        let vocabulary = ["un", "deux", "trois", "quatre", "cinq", "six", "sept"];
        for _ in 0..200 {
            let codex: Vec<&str> = (0..next(30))
                .map(|_| vocabulary[next(7) as usize])
                .collect();
            let local: Vec<&str> = (0..next(30))
                .map(|_| vocabulary[next(7) as usize])
                .collect();
            let steps = align(&codex, &local).unwrap();
            assert_eq!(codex_order(&steps), (0..codex.len()).collect::<Vec<_>>());
            let locals: Vec<usize> = steps
                .iter()
                .filter_map(|step| match step {
                    Step::Pair(_, l) | Step::Local(l) => Some(*l),
                    Step::Codex(_) => None,
                })
                .collect();
            assert_eq!(locals, (0..local.len()).collect::<Vec<_>>());
        }
    }
}
