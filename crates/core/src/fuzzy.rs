//! A small, dependency-free fuzzy scorer for command-bar autocomplete.
//!
//! [`fuzzy_match`] scores a query against a candidate string with a
//! Smith-Waterman-style dynamic program: every query character must appear in
//! order (a subsequence match), and among all valid alignments the one with the
//! highest score wins. The scoring rewards the things that make a match feel
//! "right" in a URL bar — a run of adjacent characters, and matches that land on
//! a word boundary (the string start, or just after a `.`/`/`/`-`/`_`… or a
//! camelCase hump). So `gh` prefers `**g**it**h**ub` over a scattered hit, and
//! typing the start of a host it recognises ranks that host first.
//!
//! It returns the matched character indices alongside the score so the native
//! layer can highlight exactly the characters that matched. Everything operates
//! on `char`s (Unicode scalars), not bytes, so those indices line up with how
//! the caller slices the string.
//!
//! The ranking *policy* (how a match's score combines with frecency, how a host
//! prefix outranks a mid-path hit) lives in [`crate::history`]; this module only
//! answers "how well, and where, does this query match this one string".

/// The outcome of a successful match: its score and the indices (into the
/// candidate's `char` sequence) of the characters that matched, ascending.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub score: i32,
    pub indices: Vec<u32>,
}

// Scoring weights. Tuned so a boundary-aligned or consecutive match clearly
// beats a scattered one, while the gap penalties keep long-range matches from
// scoring as well as tight ones.
const SCORE_MATCH: i32 = 16;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CONSECUTIVE: i32 = 8;
/// Extra reward for matching at the very first character, so an otherwise-equal
/// match anchored at the string start edges out one anchored after a separator.
const BONUS_START: i32 = 4;
const PENALTY_GAP_START: i32 = 3;
const PENALTY_GAP_EXTEND: i32 = 1;

/// A sentinel "impossible" score. Half of `MIN` so that adding a bounded number
/// of penalties to it never underflows.
const NEG: i32 = i32::MIN / 2;

/// "No predecessor" marker for the first query char's backtrack pointer.
const NONE_POS: usize = usize::MAX;

/// The candidate of two `(score, predecessor)` options with the higher score,
/// or whichever is present. Used to pick between starting a fresh match run and
/// extending an adjacent one.
fn max_option(a: Option<(i32, usize)>, b: Option<(i32, usize)>) -> Option<(i32, usize)> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.0 >= b.0 { a } else { b }),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Lowercase a char to a single char. `char::to_lowercase` can yield several
/// chars (e.g. `ß` → `ss`); we take the first so the lowercased sequence stays
/// index-aligned with the original, which is what keeps the returned indices
/// valid against the caller's untouched string.
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// True if candidate position `j` starts a new "word": the string start, a
/// character right after a non-alphanumeric separator, or a lower→upper camel
/// hump. Boundary matches score higher because they're where a human would
/// naturally split the string (host labels, path segments, query keys).
fn is_boundary(orig: &[char], j: usize) -> bool {
    if j == 0 {
        return true;
    }
    let prev = orig[j - 1];
    let cur = orig[j];
    !prev.is_alphanumeric() || (prev.is_lowercase() && cur.is_uppercase())
}

/// Score `query` against `text` (both matched case-insensitively). Returns
/// `None` when `query` is empty or is not a subsequence of `text`.
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    let q: Vec<char> = query.chars().map(lower).collect();
    let orig: Vec<char> = text.chars().collect();
    let t: Vec<char> = orig.iter().map(|&c| lower(c)).collect();
    let (n, m) = (q.len(), t.len());
    if n == 0 || n > m {
        return None;
    }

    // Cheap subsequence pre-check: if the query can't even be threaded through
    // the text in order, skip the O(n·m) DP entirely (the common case for most
    // candidates when the user has typed a few characters).
    {
        let mut qi = 0;
        for &c in &t {
            if qi < n && c == q[qi] {
                qi += 1;
            }
        }
        if qi < n {
            return None;
        }
    }

    // Per-position boundary bonus, precomputed once, with an extra nudge for the
    // string start.
    let bonus: Vec<i32> = (0..m)
        .map(|j| {
            let mut b = if is_boundary(&orig, j) {
                BONUS_BOUNDARY
            } else {
                0
            };
            if j == 0 {
                b += BONUS_START;
            }
            b
        })
        .collect();

    // Two DP grids over the n×m space, the standard formulation for a fuzzy
    // matcher with a consecutive-run bonus. The streak bonus makes the problem
    // non-local — a fresh match can beat a gap here because it unlocks a longer
    // adjacent run *later* — so a single "best score" per cell is not enough.
    //
    //   `mscore[i][j]` — best score for query[0..=i] with query[i] matched
    //                    *exactly at* text[j] (or NEG if impossible).
    //   `mprev[i][j]`  — where query[i-1] matched in that alignment (backtrack).
    //   `hscore[i][j]` — best score for query[0..=i] over text[0..=j], whether
    //                    the alignment ends in a match or a trailing gap.
    //   `harg[i][j]`   — where query[i] matched in that best-so-far alignment.
    //   `hgap[i][j]`   — whether hscore[i][j] came from a gap (to charge the
    //                    cheaper gap-extend penalty on the next skip).
    //
    // Consecutive runs are rewarded by letting `mscore[i][j]` extend from the
    // *match* score `mscore[i-1][j-1]` (adjacent), not just the best-so-far.
    let mut mscore = vec![NEG; n * m];
    let mut mprev = vec![NONE_POS; n * m];
    let mut hscore = vec![NEG; n * m];
    let mut harg = vec![0usize; n * m];
    let mut hgap = vec![false; n * m];

    for i in 0..n {
        for j in 0..m {
            let idx = i * m + j;

            // ── mscore: query[i] matched at text[j] ──
            if q[i] == t[j] {
                // Start a fresh run from the best alignment of query[0..i]
                // ending before j (or from nothing, for the first query char).
                let from_gap = if i == 0 {
                    Some((SCORE_MATCH + bonus[j], NONE_POS))
                } else if j > 0 && hscore[(i - 1) * m + (j - 1)] > NEG {
                    let prev = (i - 1) * m + (j - 1);
                    Some((hscore[prev] + SCORE_MATCH + bonus[j], harg[prev]))
                } else {
                    None
                };
                // Or continue a run: query[i-1] matched at j-1. Adjacent matches
                // earn at least the streak bonus (or the boundary bonus if
                // larger), which is what makes tight matches win.
                let from_match = if i > 0 && j > 0 && mscore[(i - 1) * m + (j - 1)] > NEG {
                    Some((
                        mscore[(i - 1) * m + (j - 1)]
                            + SCORE_MATCH
                            + bonus[j].max(BONUS_CONSECUTIVE),
                        j - 1,
                    ))
                } else {
                    None
                };
                if let Some((score, prev)) = max_option(from_match, from_gap) {
                    mscore[idx] = score;
                    mprev[idx] = prev;
                }
            }

            // ── hscore: best of "matched here" vs "skipped text[j]" ──
            let gap = if j > 0 && hscore[i * m + (j - 1)] > NEG {
                let pen = if hgap[i * m + (j - 1)] {
                    PENALTY_GAP_EXTEND
                } else {
                    PENALTY_GAP_START
                };
                hscore[i * m + (j - 1)] - pen
            } else {
                NEG
            };
            if mscore[idx] >= gap && mscore[idx] > NEG {
                hscore[idx] = mscore[idx];
                harg[idx] = j;
                hgap[idx] = false;
            } else {
                hscore[idx] = gap;
                harg[idx] = if j > 0 { harg[i * m + (j - 1)] } else { 0 };
                hgap[idx] = true;
            }
        }
    }

    // The best full-query alignment is the maximum over the last row — trailing
    // gaps after the final match only subtract, so the peak sits exactly at the
    // column where the last query char matched.
    let last = (n - 1) * m;
    let mut best = NEG;
    let mut best_col = 0;
    for col in 0..m {
        if hscore[last + col] > best {
            best = hscore[last + col];
            best_col = col;
        }
    }
    if best <= NEG {
        return None;
    }

    // Backtrack via the per-match predecessor pointers. `harg` maps the peak
    // column to the last query char's actual match position.
    let mut indices = Vec::with_capacity(n);
    let mut i = n - 1;
    let mut j = harg[last + best_col];
    loop {
        indices.push(j as u32);
        let prev = mprev[i * m + j];
        if i == 0 || prev == NONE_POS {
            break;
        }
        i -= 1;
        j = prev;
    }
    indices.reverse();

    Some(Match {
        score: best,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> i32 {
        fuzzy_match(query, text).map(|m| m.score).unwrap_or(NEG)
    }

    fn indices(query: &str, text: &str) -> Vec<u32> {
        fuzzy_match(query, text)
            .map(|m| m.indices)
            .unwrap_or_default()
    }

    #[test]
    fn requires_subsequence() {
        assert!(fuzzy_match("xyz", "github.com").is_none());
        assert!(fuzzy_match("bta", "abc").is_none()); // out of order
        assert!(fuzzy_match("", "github.com").is_none()); // empty query
        assert!(fuzzy_match("toolong", "abc").is_none()); // query longer than text
    }

    #[test]
    fn matches_are_case_insensitive() {
        assert!(fuzzy_match("GIT", "github.com").is_some());
        assert!(fuzzy_match("git", "GitHub.com").is_some());
    }

    #[test]
    fn returns_matched_indices() {
        // "gh" over "github" → 'g'@0, 'h'@3.
        assert_eq!(indices("gh", "github"), vec![0, 3]);
        // A contiguous prefix highlights the first N chars.
        assert_eq!(indices("git", "github"), vec![0, 1, 2]);
    }

    #[test]
    fn prefers_the_tightest_alignment() {
        // "abc" can thread through this string many ways; the optimal alignment
        // is the trailing contiguous "abc", not the scattered leading one.
        assert_eq!(indices("abc", "a_b_ababc"), vec![6, 7, 8]);
    }

    #[test]
    fn boundary_matches_beat_interior_matches() {
        // "com" at the start of a label (after '.') beats "com" buried mid-word.
        assert!(score("com", "example.community") > score("com", "welcome.net"));
    }

    #[test]
    fn consecutive_beats_scattered() {
        // A tight "hub" beats the same letters spread across the string.
        assert!(score("hub", "hub.io") > score("hub", "h-u-b.io"));
    }

    #[test]
    fn prefix_beats_deep_match() {
        // Typing the start of the host scores higher than the same letters
        // appearing later in a path.
        assert!(score("git", "github.com") > score("git", "example.com/git"));
    }

    #[test]
    fn acronym_of_camel_humps() {
        // camelCase humps are boundaries: "gh" matches the two capitals.
        assert_eq!(indices("gh", "GitHub"), vec![0, 3]);
    }
}
