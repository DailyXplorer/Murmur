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
pub fn pair_cost(a: &str, b: &str) -> f32 {
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

/// How the cheapest alignment reached a cell: by the step into it from a cell
/// whose last Codex word was paired, from one whose last was not, or by
/// skipping an on-device word.
const AFTER_PAIRED: u8 = 0;
const AFTER_UNPAIRED: u8 = 1;
const FROM_LOCAL: u8 = 2;

/// The cheaper of a cell left with its last Codex word paired and unpaired.
fn cheaper(paired: f32, unpaired: f32) -> (f32, u8) {
    if paired <= unpaired {
        (paired, AFTER_PAIRED)
    } else {
        (unpaired, AFTER_UNPAIRED)
    }
}

/// Aligns `codex` against `local` (comparison keys). Every Codex word appears
/// exactly once, in order. `None` when the inputs are too large.
pub fn align(codex: &[&str], local: &[&str]) -> Option<Vec<Step>> {
    align_by(codex.len(), local.len(), 0.0, |i, j| {
        pair_cost(codex[i], local[j])
    })
}

/// `align` for `n` Codex words and `m` on-device words, where pairing Codex
/// word `i` with on-device word `j` costs `pair_cost(i, j)`, and each run of
/// paired Codex words costs `run_cost` more: of two alignments otherwise
/// equal, the one that pairs fewer, longer runs wins.
pub fn align_by(
    n: usize,
    m: usize,
    run_cost: f32,
    pair_cost: impl Fn(usize, usize) -> f32,
) -> Option<Vec<Step>> {
    if (n + 1).saturating_mul(m + 1) > MAX_CELLS {
        return None;
    }
    let width = m + 1;
    // Per cell, how the cheapest alignment that ends there with its last
    // Codex word paired (low bits), and unpaired (high bits), got there.
    let mut from = vec![FROM_LOCAL | (FROM_LOCAL << 2); (n + 1) * width];
    // Leading on-device words cost nothing.
    let mut previous_paired = vec![f32::INFINITY; width];
    let mut previous_unpaired = vec![0.0f32; width];
    let mut paired = vec![f32::INFINITY; width];
    let mut unpaired = vec![0.0f32; width];
    for i in 1..=n {
        unpaired[0] = i as f32 * CODEX_GAP;
        from[i * width] = FROM_LOCAL | (AFTER_UNPAIRED << 2);
        for j in 1..=m {
            let (before, after) =
                cheaper(previous_paired[j - 1], previous_unpaired[j - 1] + run_cost);
            let pair = before + pair_cost(i - 1, j - 1);
            let skip_local = paired[j - 1] + LOCAL_GAP;
            let paired_from = if pair <= skip_local {
                paired[j] = pair;
                after
            } else {
                paired[j] = skip_local;
                FROM_LOCAL
            };
            let (before, after) = cheaper(previous_paired[j], previous_unpaired[j]);
            let skip_codex = before + CODEX_GAP;
            let skip_local = unpaired[j - 1] + LOCAL_GAP;
            let unpaired_from = if skip_codex <= skip_local {
                unpaired[j] = skip_codex;
                after
            } else {
                unpaired[j] = skip_local;
                FROM_LOCAL
            };
            from[i * width + j] = paired_from | (unpaired_from << 2);
        }
        std::mem::swap(&mut previous_paired, &mut paired);
        std::mem::swap(&mut previous_unpaired, &mut unpaired);
    }
    // Trailing on-device words cost nothing either: end where it is cheapest.
    let end = |j: usize| previous_paired[j].min(previous_unpaired[j]);
    let mut j = (0..=m)
        .min_by(|a, b| end(*a).total_cmp(&end(*b)))
        .unwrap_or(0);
    let mut last_paired = previous_paired[j] <= previous_unpaired[j];
    let mut steps: Vec<Step> = (j..m).rev().map(Step::Local).collect();
    let mut i = n;
    while i > 0 || j > 0 {
        if i == 0 {
            j -= 1;
            steps.push(Step::Local(j));
            continue;
        }
        let shift = if last_paired { 0 } else { 2 };
        match (from[i * width + j] >> shift) & 3 {
            FROM_LOCAL => {
                j -= 1;
                steps.push(Step::Local(j));
            }
            after => {
                i -= 1;
                if last_paired {
                    j -= 1;
                    steps.push(Step::Pair(i, j));
                } else {
                    steps.push(Step::Codex(i));
                }
                last_paired = after == AFTER_PAIRED;
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
    fn a_run_cost_pairs_the_copy_that_runs_on() {
        let paired = |codex: &str, local: &str| -> Vec<(usize, usize)> {
            let (codex, local) = (words(codex), words(local));
            align_by(codex.len(), local.len(), 0.0001, |i, j| {
                pair_cost(codex[i], local[j])
            })
            .unwrap()
            .into_iter()
            .filter_map(|step| match step {
                Step::Pair(c, l) => Some((c, l)),
                _ => None,
            })
            .collect()
        };
        assert_eq!(
            paired("dix heures dix heures parfait", "dix heures parfait"),
            [(2, 0), (3, 1), (4, 2)]
        );
        assert_eq!(
            paired("à dix heures dix heures", "à dix heures"),
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
