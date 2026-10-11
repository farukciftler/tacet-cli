//! Turning a score difference into a decision.
//!
//! WHY IT EXISTS: every "+3 points" claim in this project has so far been read
//! off two table headers. With 115 English and 69 Turkish tool cases, one case
//! is worth 0.87 and 1.45 points — so "+3 points" can mean two or three cases,
//! and on the day this was written it meant ONE (the suites were 32 and 18).
//! One case is not evidence of anything. This module is the smallest set of
//! functions that answers "is this difference real, and how many cases would I
//! need for it to be answerable at all".
//!
//! NO DEPENDENCY WAS ADDED. The randomness is a hand-written xorshift64*, the
//! statistics are a sign test, a paired bootstrap and Mann-Whitney U — each a
//! few dozen lines whose correctness is pinned by hand-computed fixtures in the
//! tests. The same criterion as the SHA-256 in the kernel: a closed subset,
//! provable against known values.
//!
//! EVERYTHING IS DETERMINISTIC. The bootstrap takes a seed and the same seed
//! gives the same interval, forever — a confidence interval that moves when you
//! rerun it is not a measurement, it is a mood.

/// The one decision rule this project needs most often.
///
/// Two arms are compared CASE BY CASE. Pairs where both arms agree carry no
/// information (that is the whole point of a paired test); what remains is
/// `fixed` (arm B got it right where A did not) and `broken` (the reverse).
/// Under the null each discordant pair is a coin flip, so the p-value is the
/// exact two-sided binomial tail.
///
/// THE NUMBER WORTH REMEMBERING: with an instrument that returns the same
/// answer on a rerun — this one does — a difference is only callable at 95%
/// when SIX pairs move one way and none the other (2 × 0.5⁶ = 0.031). Five
/// one-way pairs give 0.0625: suggestive, not significant.
///
/// SIX IS A PROPERTY OF THE TEST, NOT OF THE SUITE, and that distinction is the
/// one this comment used to blur: growing the suite does not lower the threshold,
/// it only makes six cases worth fewer POINTS. On the 184-case pooled selection
/// suite six cases is 3.3 points (it was 12 on the 50-case suite this line was
/// written for); on the 78-case behavioural suite it is 7.7. So "+2 points" is
/// still not a threshold anybody can meet here — see `cases_needed`.
pub fn sign_test(fixed: usize, broken: usize) -> f64 {
    let n = fixed + broken;
    if n == 0 {
        // Nothing moved. That is not evidence of no effect, it is no evidence.
        return 1.0;
    }
    let smaller = fixed.min(broken);
    // C(n, k) · 0.5^n, walked from k = 0 by the ratio (n - k) / (k + 1).
    //
    // IN LOG SPACE ON PURPOSE: C(1000, 500) overflows an f64 while 0.5^1000
    // underflows it, and the product is a perfectly ordinary number.
    // Multiplying them directly gives `inf * 0 = NaN` — a p-value that silently
    // becomes NaN is worse than no p-value.
    //
    // WALKED RATHER THAN RECOMPUTED PER TERM: `cases_needed_observed` asks this
    // for suites of tens of thousands of cases, and three O(n) log-factorials
    // per term made each call quadratic.
    let mut log_pmf = n as f64 * 0.5f64.ln();
    let mut tail = 0.0;
    for k in 0..=smaller {
        tail += log_pmf.exp();
        log_pmf += ((n - k) as f64).ln() - ((k + 1) as f64).ln();
    }
    (2.0 * tail).min(1.0)
}

/// How many paired cases a suite needs before an effect of `points` (in
/// percentage points, e.g. 3.0) could be called at 95% two-sided.
///
/// Assumes the effect shows up as one-way discordant pairs, which is what a
/// deterministic instrument gives: a real improvement moves cases from wrong to
/// right and moves nothing back. Six such pairs are needed, so the suite must
/// be large enough that `points` percent of it is six cases.
pub fn cases_needed(points: f64) -> usize {
    if points <= 0.0 {
        return usize::MAX;
    }
    (600.0 / points).ceil() as usize
}

/// How many paired cases a suite needs to call, at 95%, the effect a
/// comparison ACTUALLY OBSERVED — with the cases it broke counted, not only the
/// ones it fixed.
///
/// WHY `cases_needed` IS NOT ENOUGH FOR THIS (audit L30). That function answers
/// for a one-way effect: every discordant pair in the same direction. A real
/// comparison is two-sided. `--compare` read fixed 10 / broke 4 on 100 cases,
/// p = 0.1796, NOT DISTINGUISHABLE — and then printed "this instrument needs 100
/// paired cases to call a 6.0-point effect; this suite has 100", a sentence that
/// contradicts the verdict above it. The four broken cases are noise the effect
/// has to be seen through, and only a larger suite does that.
///
/// The answer is the smallest suite on which the SAME fixed and broken rates
/// would pass the sign test, to within the rounding of a case count. `None`
/// when no suite would: as many cases broke as were fixed, or a zero-case run.
/// Searched up to a million cases; past that, `None` as well, because "more
/// than a million" is not a suite anyone is going to write.
pub fn cases_needed_observed(fixed: usize, broken: usize, n: usize) -> Option<usize> {
    const LIMIT: usize = 1_000_000;
    if n == 0 || fixed == broken {
        return None;
    }
    let p_at = |m: usize| {
        let scale = m as f64 / n as f64;
        sign_test(
            (fixed as f64 * scale).round() as usize,
            (broken as f64 * scale).round() as usize,
        )
    };
    if p_at(n) < 0.05 {
        return Some(n);
    }
    // Doubling to an upper bound, then halving down to the smallest. The
    // p-value is not strictly monotone in `m` — the counts are rounded — so this
    // finds the boundary, not every isolated size that happens to dip under it.
    let mut low = n;
    let mut high = n;
    loop {
        if high >= LIMIT {
            return None;
        }
        high = (high * 2).min(LIMIT);
        if p_at(high) < 0.05 {
            break;
        }
        low = high;
    }
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if p_at(mid) < 0.05 {
            high = mid;
        } else {
            low = mid;
        }
    }
    Some(high)
}

/// The paired difference and its interval, by resampling CASES (not steps).
///
/// Returns `(delta, low, high)` as fractions of the suite — multiply by 100 for
/// points. `pairs` is one entry per case: `(arm_a_passed, arm_b_passed)`.
///
/// WHY CASES AND NOT STEPS: a multi-turn case that breaks on its first step
/// fails every later step too, so steps are not independent draws. Resampling
/// them would narrow the interval by pretending three correlated observations
/// are three independent ones.
pub fn paired_bootstrap(pairs: &[(bool, bool)], resamples: usize, seed: u64) -> (f64, f64, f64) {
    let n = pairs.len();
    if n == 0 || resamples == 0 {
        return (0.0, 0.0, 0.0);
    }
    let delta = mean_delta(pairs);
    let mut rng = Xorshift64Star::new(seed);
    let mut deltas: Vec<f64> = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut sum = 0.0;
        for _ in 0..n {
            let i = (rng.draw() % n as u64) as usize;
            let (a, b) = pairs[i];
            sum += f64::from(b) - f64::from(a);
        }
        deltas.push(sum / n as f64);
    }
    deltas.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a bootstrap"));
    let low = percentile(&deltas, 2.5);
    let high = percentile(&deltas, 97.5);
    (delta, low, high)
}

fn mean_delta(pairs: &[(bool, bool)]) -> f64 {
    let sum: f64 = pairs
        .iter()
        .map(|(a, b)| f64::from(*b) - f64::from(*a))
        .sum();
    sum / pairs.len() as f64
}

/// The area under the ROC curve, by Mann-Whitney U with tie-averaged ranks.
///
/// `positive` are the feature's values on the cases with the property being
/// predicted (for us: the decisions that came out WRONG), `negative` the rest.
/// 0.5 means the feature carries no signal; 1.0 means it separates perfectly.
pub fn auroc(positive: &[f64], negative: &[f64]) -> f64 {
    if positive.is_empty() || negative.is_empty() {
        return 0.5;
    }
    let mut all: Vec<(f64, bool)> = positive
        .iter()
        .map(|v| (*v, true))
        .chain(negative.iter().map(|v| (*v, false)))
        .collect();
    all.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("no NaN in a feature"));

    // Tie-averaged ranks: a feature that is constant must come out at exactly
    // 0.5 rather than at whatever the sort order happened to be.
    let mut ranks = vec![0.0f64; all.len()];
    let mut i = 0;
    while i < all.len() {
        let mut j = i;
        while j + 1 < all.len() && all[j + 1].0 == all[i].0 {
            j += 1;
        }
        let average = ((i + j) as f64) / 2.0 + 1.0;
        for rank in ranks.iter_mut().take(j + 1).skip(i) {
            *rank = average;
        }
        i = j + 1;
    }

    let rank_sum: f64 = ranks
        .iter()
        .zip(all.iter())
        .filter(|(_, (_, is_positive))| *is_positive)
        .map(|(r, _)| *r)
        .sum();
    let (m, n) = (positive.len() as f64, negative.len() as f64);
    let u = rank_sum - m * (m + 1.0) / 2.0;
    u / (m * n)
}

/// The Hanley-McNeil standard error of an AUROC, and its 95% interval.
///
/// THE BINDING CONSTRAINT IS THE SMALLER CLASS. With 46 correct decisions and 8
/// wrong ones, an AUROC of 0.75 carries a lower bound around 0.59 — so a point
/// estimate landing exactly on a 0.75 threshold still fails a gate phrased as
/// "CI lower bound above 0.65". Growing the suite helps only insofar as it
/// produces more WRONG decisions, which is the uncomfortable part: the better
/// the model gets, the harder its own uncertainty is to measure.
pub fn auroc_interval(area: f64, positives: usize, negatives: usize) -> (f64, f64, f64) {
    if positives == 0 || negatives == 0 {
        return (area, 0.0, 1.0);
    }
    let (m, n) = (positives as f64, negatives as f64);
    let q1 = area / (2.0 - area);
    let q2 = 2.0 * area * area / (1.0 + area);
    let variance =
        (area * (1.0 - area) + (m - 1.0) * (q1 - area * area) + (n - 1.0) * (q2 - area * area))
            / (m * n);
    let se = variance.max(0.0).sqrt();
    (se, (area - 1.96 * se).max(0.0), (area + 1.96 * se).min(1.0))
}

/// Clopper-Pearson-ish honesty for a rate at its ceiling.
///
/// WHY IT IS HERE: this project reports IRRELEVANCE as 6/6 and 4/4 and calls it
/// 100%, with a guard saying it "must not drop". Six out of six has a 95% lower
/// bound of 0.54 — "100%" honestly reads "somewhere between 54% and 100%", and
/// a guard against a drop from a number like that cannot resolve better than
/// half the scale.
///
/// This is the TWO-SIDED 95% interval's lower end, which for k = n is exactly
/// `(alpha/2)^(1/n)` and needs no iteration. Two-sided because that is what a
/// "95% confidence interval" means everywhere else in this file; the one-sided
/// bound (`alpha^(1/n)`, 0.61 at n = 6) is the friendlier number and is not the
/// one being claimed.
pub fn perfect_score_lower_bound(n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    0.025f64.powf(1.0 / n as f64)
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// xorshift64* — small, fast, and good enough for resampling indices.
///
/// NOT FOR CRYPTOGRAPHY, and nothing here pretends otherwise. It exists so the
/// bootstrap is reproducible without pulling in a random-number crate.
pub struct Xorshift64Star {
    state: u64,
}

impl Xorshift64Star {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point of xorshift; the constant is the one
        // the reference implementation uses for exactly this reason.
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    /// Named `draw`, not `next`: a bare `next` on a non-iterator reads like
    /// `Iterator::next` at the call site and clippy says so.
    pub fn draw(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        return sorted[low];
    }
    let weight = rank - low as f64;
    sorted[low] * (1.0 - weight) + sorted[high] * weight
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-term formula `sign_test` used before it walked the terms, kept
    /// as the reference the walk must reproduce.
    fn binomial_pmf(n: usize, k: usize, p: f64) -> f64 {
        let log_factorial = |n: usize| (2..=n).map(|i| (i as f64).ln()).sum::<f64>();
        let log = log_factorial(n) - log_factorial(k) - log_factorial(n - k)
            + (k as f64) * p.ln()
            + ((n - k) as f64) * (1.0 - p).ln();
        log.exp()
    }

    #[test]
    fn the_walked_tail_is_the_per_term_tail() {
        for fixed in 0..40 {
            for broken in 0..40 {
                let n = fixed + broken;
                if n == 0 {
                    continue;
                }
                let tail: f64 = (0..=fixed.min(broken))
                    .map(|k| binomial_pmf(n, k, 0.5))
                    .sum();
                let want = (2.0 * tail).min(1.0);
                assert!(
                    (sign_test(fixed, broken) - want).abs() < 1e-9,
                    "{fixed}/{broken}: {} vs {want}",
                    sign_test(fixed, broken)
                );
            }
        }
        // And it does not turn into NaN where the naive product would.
        assert!(sign_test(600, 400).is_finite());
    }

    /// THE AUDIT'S OWN NUMBERS (L30): fixed 10 / broke 4 / 100 is p = 0.1796,
    /// so whatever suite would call it, it is not the 100 cases that just
    /// failed to.
    #[test]
    fn a_two_sided_effect_needs_more_than_the_suite_that_could_not_call_it() {
        assert!(sign_test(10, 4) > 0.05);
        let needed = cases_needed_observed(10, 4, 100).expect("a net effect is callable somewhere");
        assert!(needed > 100, "{needed}");
        // And the answer is a suite on which the same rates DO pass.
        let at = |m: usize| sign_test((10 * m).div_ceil(100), (4 * m) / 100);
        assert!(at(needed + 10) < 0.05, "{needed}");
        // The broken cases cost: the one-way bound for the same net 6 points is
        // the 100 that `cases_needed` printed.
        assert_eq!(cases_needed(6.0), 100);
    }

    #[test]
    fn a_one_way_effect_lands_where_the_one_way_bound_says() {
        // 3 of 100, all one way: six such pairs need about 200 cases.
        let needed = cases_needed_observed(3, 0, 100).unwrap();
        assert!((101..=cases_needed(3.0)).contains(&needed), "{needed}");
        // Already callable: the suite it was measured on is enough.
        assert_eq!(cases_needed_observed(6, 0, 50), Some(50));
    }

    /// The numbers of the two `--compare` outputs the README quotes. 0 fixed /
    /// 3 broke / 184 printed 368 under the one-way formula; 8 fixed / 5 broke /
    /// 115 printed 230, which ignored the five broken cases entirely. Whether
    /// the PAGE says so is checked where the sentence is printed:
    /// `eval_cmd::tests::the_resolution_lines_the_readme_quotes_are_what_compare_prints`
    /// in tacet-cli, which reads the README.
    #[test]
    fn the_resolution_numbers_of_the_two_readme_blocks() {
        assert_eq!(cases_needed_observed(0, 3, 184), Some(338));
        assert_eq!(cases_needed_observed(8, 5, 115), Some(712));
    }

    #[test]
    fn no_suite_calls_a_wash() {
        assert_eq!(cases_needed_observed(5, 5, 100), None);
        assert_eq!(cases_needed_observed(0, 0, 100), None);
        assert_eq!(cases_needed_observed(1, 0, 0), None);
        // 51 against 49 needs far more than any suite here (9 719 by this
        // search, 9 604 by the normal approximation), and the search
        // must end rather than walk to infinity.
        let tiny = cases_needed_observed(51, 49, 100);
        assert!(tiny.is_none_or(|m| m > 5_000), "{tiny:?}");
    }

    #[test]
    fn the_sign_test_matches_hand_computed_values() {
        // Nothing moved: no evidence, and explicitly not "no effect".
        assert_eq!(sign_test(0, 0), 1.0);
        // One case moved: 2 × 0.5 = 1.0.
        assert!((sign_test(1, 0) - 1.0).abs() < 1e-12);
        // Five one way: 2 × 0.5^5 = 0.0625. SUGGESTIVE, NOT SIGNIFICANT — the
        // number this project's own review first quoted as significant, which
        // was the one-sided value.
        assert!((sign_test(5, 0) - 0.0625).abs() < 1e-12);
        // Six one way: 2 × 0.5^6 = 0.03125. This is the real threshold.
        assert!(sign_test(6, 0) < 0.05);
        // Symmetric, and a wash is a wash.
        assert!((sign_test(0, 6) - sign_test(6, 0)).abs() < 1e-12);
        assert!((sign_test(3, 3) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_suite_size_a_claim_needs() {
        // "+3 points" needs six cases to be three percent of the suite.
        assert_eq!(cases_needed(3.0), 200);
        assert_eq!(cases_needed(2.0), 300);
        assert_eq!(cases_needed(1.0), 600);
        // THE SUITE SIZES ARE READ, NOT WRITTEN DOWN. This block used to say
        // `cases_needed(12) <= 50` — and 50 was the pooled size ON THE DAY IT WAS
        // WRITTEN. The suites grew to 184 and this test went on passing while the
        // comment above it went on saying 50, which is exactly the drift the
        // module exists to argue against. Derived from the real suites, the
        // number in the prose and the number in the code cannot disagree again.
        for suite in [
            crate::all().len(),
            crate::selection_cases().len() + crate::turkish_selection_cases().len(),
        ] {
            let smallest = 600.0 / suite as f64;
            // A hair above the smallest callable effect is callable; a hair
            // below is not. The 0.01 keeps floating point out of the claim.
            assert!(
                cases_needed(smallest + 0.01) <= suite,
                "{smallest:.2} points should be callable on {suite} cases"
            );
            assert!(
                cases_needed(smallest - 0.01) > suite,
                "{smallest:.2} points is the FLOOR on {suite} cases, not a step above it"
            );
        }
        // Measured today, for whoever reads the numbers in the docs: 78
        // behavioural cases (7.7 points) and 184 pooled selection cases (3.3).
        assert!(600.0 / crate::all().len() as f64 <= 12.0);
    }

    #[test]
    fn the_bootstrap_is_reproducible_and_brackets_the_truth() {
        // Arm B fixes ten cases out of a hundred and breaks none.
        let pairs: Vec<(bool, bool)> = (0..100).map(|i| (i >= 10, true)).collect();
        let (delta, low, high) = paired_bootstrap(&pairs, 2000, 42);
        assert!((delta - 0.10).abs() < 1e-12);
        assert!(
            low > 0.0,
            "an interval that includes zero would be wrong here"
        );
        assert!(high > delta);
        // Same seed, same interval — forever.
        let again = paired_bootstrap(&pairs, 2000, 42);
        assert_eq!((low, high), (again.1, again.2));
        // A different seed moves it a little, not a lot.
        let other = paired_bootstrap(&pairs, 2000, 7);
        assert!((other.1 - low).abs() < 0.05);
    }

    #[test]
    fn a_wash_bootstrap_includes_zero() {
        let pairs: Vec<(bool, bool)> = (0..100).map(|i| (i % 2 == 0, i % 2 == 0)).collect();
        let (delta, low, high) = paired_bootstrap(&pairs, 1000, 1);
        assert_eq!(delta, 0.0);
        assert!(low <= 0.0 && high >= 0.0);
    }

    #[test]
    fn auroc_matches_hand_computed_cases() {
        // Perfect separation.
        assert!((auroc(&[3.0, 4.0, 5.0], &[0.0, 1.0, 2.0]) - 1.0).abs() < 1e-12);
        // Perfectly reversed.
        assert!((auroc(&[0.0, 1.0], &[2.0, 3.0]) - 0.0).abs() < 1e-12);
        // A constant feature carries nothing — this is what tie-averaging is
        // for; without it the answer would depend on sort order.
        assert!((auroc(&[1.0, 1.0, 1.0], &[1.0, 1.0]) - 0.5).abs() < 1e-12);
        // Hand-computed: positives {2,4}, negatives {1,3,5}. Pairs where the
        // positive wins: (2>1), (4>1), (4>3) = 3 of 6.
        assert!((auroc(&[2.0, 4.0], &[1.0, 3.0, 5.0]) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn the_auroc_interval_shows_what_eight_errors_buy() {
        // The situation this project is actually in: 8 wrong decisions, 46
        // right ones. A 0.75 point estimate cannot clear a 0.65 lower bound.
        let (se, low, _high) = auroc_interval(0.75, 8, 46);
        assert!(se > 0.07, "se was {se}");
        assert!(
            low < 0.65,
            "a gate demanding a 0.65 lower bound is unreachable at this error count: {low}"
        );
    }

    #[test]
    fn a_perfect_rate_is_not_certainty() {
        // 6/6 reads "between 54% and 100%" — the number this project published.
        let bound = perfect_score_lower_bound(6);
        assert!((bound - 0.5407).abs() < 1e-3, "{bound}");
        // 4/4, the Turkish irrelevance denominator, is weaker still.
        assert!((perfect_score_lower_bound(4) - 0.3976).abs() < 1e-3);
        // It takes 72 clean cases before "100%" means "at least 95%". The
        // irrelevance sets are 6 and 4.
        assert!(perfect_score_lower_bound(72) >= 0.95);
        assert!(perfect_score_lower_bound(71) < 0.95);
    }

    #[test]
    fn the_generator_does_not_get_stuck() {
        let mut zero = Xorshift64Star::new(0);
        let first = zero.draw();
        assert_ne!(first, 0);
        assert_ne!(first, zero.draw());
        // Reproducible.
        let mut a = Xorshift64Star::new(99);
        let mut b = Xorshift64Star::new(99);
        for _ in 0..100 {
            assert_eq!(a.draw(), b.draw());
        }
    }
}
