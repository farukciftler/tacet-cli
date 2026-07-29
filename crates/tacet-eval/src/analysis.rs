//! Turning a score difference into a decision.
//!
//! WHY IT EXISTS: every "+3 points" claim in this project has so far been read
//! off two table headers. With 32 English and 18 Turkish tool cases, one case
//! is worth 3.13 and 5.56 points — so "+3 points" can mean ONE case, and one
//! case is not evidence of anything. This module is the smallest set of
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
/// one-way pairs give 0.0625: suggestive, not significant. On a 50-case pooled
/// suite six cases is 12 points, which is why "+2 points" is not a threshold
/// anybody can meet here — see `cases_needed`.
pub fn sign_test(fixed: usize, broken: usize) -> f64 {
    let n = fixed + broken;
    if n == 0 {
        // Nothing moved. That is not evidence of no effect, it is no evidence.
        return 1.0;
    }
    let smaller = fixed.min(broken);
    let tail: f64 = (0..=smaller).map(|k| binomial_pmf(n, k, 0.5)).sum();
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

/// C(n, k) · p^k · (1-p)^(n-k), computed in log space.
///
/// IN LOG SPACE ON PURPOSE: C(1000, 500) overflows an f64 while 0.5^1000
/// underflows it, and the product is a perfectly ordinary number. Multiplying
/// them directly gives `inf * 0 = NaN` — a p-value that silently becomes NaN is
/// worse than no p-value.
fn binomial_pmf(n: usize, k: usize, p: f64) -> f64 {
    if k > n {
        return 0.0;
    }
    let log = log_factorial(n) - log_factorial(k) - log_factorial(n - k)
        + (k as f64) * p.ln()
        + ((n - k) as f64) * (1.0 - p).ln();
    log.exp()
}

fn log_factorial(n: usize) -> f64 {
    (2..=n).map(|i| (i as f64).ln()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Today's pooled suite is 50 cases, so the smallest callable claim is
        // 12 points — which is what "the gates are tighter than the instrument"
        // means in one number.
        assert!(cases_needed(12.0) <= 50);
        assert!(cases_needed(11.0) > 50);
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
