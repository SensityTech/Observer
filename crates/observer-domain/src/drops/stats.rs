//! The tests behind every "rising" and "falling" the drops page says.
//!
//! A category's drops in two windows are compared as two Poisson rates. With no change, the
//! recent window's drops, given the drops of both, follow a binomial whose share is the recent
//! window's share of the exposure. The caller has already collapsed bursts: a count is a count of
//! events, an incident counting once.

use std::f64::consts::FRAC_1_SQRT_2;
use std::sync::LazyLock;

/// The p-value under which a single comparison is a change.
pub const SIGNIFICANCE: f64 = 0.01;
/// The fewest drops, both windows together, a verdict needs.
pub const MIN_EVENTS: u64 = 20;
/// The smallest change of rate worth a word: 15 % either way.
pub const MIN_CHANGE: f64 = 0.15;
/// The fewest drops a row of a list needs to enter the false-discovery control.
pub const LIST_MIN_EVENTS: u64 = 10;
/// The false-discovery rate a list of comparisons is held to.
pub const FALSE_DISCOVERY: f64 = 0.05;

/// The most drops the exact test sums over; above, the normal approximation is as good.
const EXACT_MAX_EVENTS: u32 = 5_000;
/// Two splits this close in probability are a tie, as R's `binom.test` has it.
const TIE_TOLERANCE: f64 = 1e-7;
/// The normal quantile of a 95 % interval.
const Z_95: f64 = 1.96;
const MS_PER_HUNDRED_HOURS: f64 = 360_000_000.0;

/// ln k! for k up to `EXACT_MAX_EVENTS`, summed once.
static LN_FACTORIALS: LazyLock<Vec<f64>> = LazyLock::new(|| {
    let running = (1..=EXACT_MAX_EVENTS).scan(0.0, |sum, k| {
        *sum += f64::from(k).ln();
        Some(*sum)
    });
    std::iter::once(0.0).chain(running).collect()
});

/// Two windows of one category: drops now against drops before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// The drops of both windows.
    pub events: u64,
    /// The rate now over the rate before. `None` without exposure or without any drop; infinite
    /// when only the earlier window had none.
    pub ratio: Option<f64>,
    /// The 95 % interval of the ratio, `None` when the ratio is.
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// Two-sided; 1.0 when no test is possible.
    pub p: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Rising,
    Falling,
    NoClearChange,
    NotEnoughDrops,
}

/// Compares `k_a` drops over `e_a` player-hours (now) with `k_b` over `e_b` (before).
/// `dispersion` is [`dispersion`]'s: above 1 it widens the test and the interval.
pub fn compare(k_a: u64, e_a: f64, k_b: u64, e_b: f64, dispersion: f64) -> Comparison {
    let events = k_a.saturating_add(k_b);
    if !(usable(e_a) && usable(e_b)) || events == 0 {
        return Comparison {
            events,
            ratio: None,
            low: None,
            high: None,
            p: 1.0,
        };
    }
    // NaN or under 1 is no widening.
    let widened = dispersion > 1.0;
    let phi = if widened { dispersion } else { 1.0 };
    let (a, b) = (real(k_a), real(k_b));
    // The interval and the approximation add half a drop to both windows when one has none.
    let (a, b) = if k_a == 0 || k_b == 0 {
        (a + 0.5, b + 0.5)
    } else {
        (a, b)
    };
    // Logs of the parts: no quotient overflows, however far apart the exposures.
    let log_ratio = (a.ln() - e_a.ln()) - (b.ln() - e_b.ln());
    let ratio = match (k_a, k_b) {
        (0, _) => 0.0,
        (_, 0) => f64::INFINITY,
        // Through the logs only when both rates overflow or both underflow.
        _ => Some((a / e_a) / (b / e_b))
            .filter(|ratio| !ratio.is_nan())
            .unwrap_or_else(|| log_ratio.exp()),
    };
    let spread = (phi * (1.0 / a + 1.0 / b)).sqrt();
    let exact = match (usize::try_from(k_a), usize::try_from(events)) {
        (Ok(k_a), Ok(n)) if n <= EXACT_MAX_EVENTS as usize => Some(exact_p(k_a, n, e_a, e_b)),
        _ => None,
    };
    let normal = || erfc((log_ratio / spread).abs() * FRAC_1_SQRT_2);
    // A widened test is never looser than the exact one: at small counts the approximation alone
    // could call a change that the exact test would not.
    let p = match exact {
        Some(exact) if widened => exact.max(normal()),
        Some(exact) => exact,
        None => normal(),
    };
    Comparison {
        events,
        ratio: Some(ratio),
        low: Some((log_ratio - Z_95 * spread).exp()),
        high: Some((log_ratio + Z_95 * spread).exp()),
        p,
    }
}

/// The verdict of a comparison on its own.
pub fn verdict(comparison: &Comparison) -> Verdict {
    judge(comparison, comparison.p < SIGNIFICANCE)
}

/// The verdict of a comparison in a list, where [`benjamini_hochberg`] at [`FALSE_DISCOVERY`]
/// decides what is significant instead of [`SIGNIFICANCE`].
pub fn verdict_in_list(comparison: &Comparison, rejected: bool) -> Verdict {
    judge(comparison, rejected)
}

/// The verdicts of a list's rows, in its order. Only rows with [`LIST_MIN_EVENTS`] drops and a
/// test enter the false-discovery control.
pub fn list_verdicts(comparisons: &[Comparison]) -> Vec<Verdict> {
    let tested = |comparison: &Comparison| comparison.events >= LIST_MIN_EVENTS && comparison.ratio.is_some();
    let p_values: Vec<f64> = comparisons
        .iter()
        .filter(|comparison| tested(comparison))
        .map(|c| c.p)
        .collect();
    let mut falls = benjamini_hochberg(&p_values, FALSE_DISCOVERY).into_iter();
    comparisons
        .iter()
        .map(|comparison| {
            let rejected = tested(comparison) && falls.next().unwrap_or(false);
            verdict_in_list(comparison, rejected)
        })
        .collect()
}

fn judge(comparison: &Comparison, significant: bool) -> Verdict {
    let Some(ratio) = comparison.ratio else {
        return Verdict::NotEnoughDrops;
    };
    if comparison.events < MIN_EVENTS {
        Verdict::NotEnoughDrops
    } else if significant && ratio >= 1.0 + MIN_CHANGE {
        Verdict::Rising
    } else if significant && ratio <= 1.0 / (1.0 + MIN_CHANGE) {
        Verdict::Falling
    } else {
        Verdict::NoClearChange
    }
}

/// How much more the daily drops spread than a steady rate would make them: Pearson's χ² over
/// the days with exposure, divided by their number less one, and never under 1.
pub fn dispersion(days: &[(u64, f64)]) -> f64 {
    let days = || days.iter().filter(|(_, exposure)| usable(*exposure));
    let count = days().count();
    let drops: u64 = days().map(|(drops, _)| drops).sum();
    if count < 2 || drops == 0 {
        return 1.0;
    }
    let rate = real(drops) / days().map(|(_, exposure)| exposure).sum::<f64>();
    let chi_squared: f64 = days()
        .map(|&(drops, exposure)| {
            let expected = rate * exposure;
            (real(drops) - expected).powi(2) / expected
        })
        .sum();
    // `max` also turns a NaN into 1.
    (chi_squared / real(count as u64 - 1)).max(1.0)
}

/// Which hypotheses fall at false-discovery rate `q`, in the input's order: the largest p-value
/// under its rank's share of `q`, and every p-value up to it, ties included. A NaN counts as 1.
pub fn benjamini_hochberg(p_values: &[f64], q: f64) -> Vec<bool> {
    let mut sorted: Vec<f64> = p_values
        .iter()
        .map(|&p| if p.is_nan() { 1.0 } else { p })
        .collect();
    sorted.sort_by(f64::total_cmp);
    let tests = real(sorted.len() as u64);
    let cutoff = sorted
        .iter()
        .enumerate()
        .rev()
        .find(|&(index, &p)| p <= real(index as u64 + 1) * q / tests)
        .map(|(_, &p)| p);
    p_values
        .iter()
        .map(|&p| cutoff.is_some_and(|cutoff| p <= cutoff))
        .collect()
}

/// Drops per 100 player-hours; `None` without exposure.
pub fn per_hundred_hours(drops: u64, player_ms: i64) -> Option<f64> {
    let player_ms = u64::try_from(player_ms).ok().filter(|&ms| ms > 0)?;
    Some(real(drops) * MS_PER_HUNDRED_HOURS / real(player_ms))
}

/// The exact two-sided p-value of `k_a` of `n` drops falling in the window of exposure `e_a`:
/// the probability of every split no more likely than the one seen.
fn exact_p(k_a: usize, n: usize, e_a: f64, e_b: f64) -> f64 {
    // ln of each window's share of the exposure, without forming the sum.
    let ln_share_a = -(e_b / e_a).ln_1p();
    let ln_share_b = -(e_a / e_b).ln_1p();
    let ln_factorial = &*LN_FACTORIALS;
    let ln_probability = |x: usize| {
        ln_factorial[n] - ln_factorial[x] - ln_factorial[n - x]
            + times(x, ln_share_a)
            + times(n - x, ln_share_b)
    };
    let seen = ln_probability(k_a) + TIE_TOLERANCE.ln_1p();
    let p: f64 = (0..=n)
        .map(ln_probability)
        .filter(|&ln| ln <= seen)
        .map(f64::exp)
        .sum();
    p.min(1.0)
}

/// `x · ln_share`, 0 when `x` is 0 even if the share's log is −∞.
fn times(x: usize, ln_share: f64) -> f64 {
    if x == 0 { 0.0 } else { real(x as u64) * ln_share }
}

/// The complementary error function, by the Chebyshev fit of Numerical Recipes in C (2nd
/// edition, §6.2, `erfcc`): relative error under 1.2e-7 everywhere.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let fit = -1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let tail = t * (fit - x * x).exp();
    if x >= 0.0 { tail } else { 2.0 - tail }
}

fn usable(exposure: f64) -> bool {
    exposure.is_finite() && exposure > 0.0
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counts stay far below 2⁵³, where a float stops being exact"
)]
fn real(count: u64) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const EQUAL: (f64, f64) = (1.0, 1.0);

    fn assert_close(actual: f64, expected: f64, relative: f64) {
        assert!(
            (actual - expected).abs() <= relative * expected.abs(),
            "{actual} is not within {relative} of {expected}"
        );
    }

    fn compared(k_a: u64, k_b: u64, (e_a, e_b): (f64, f64)) -> Comparison {
        compare(k_a, e_a, k_b, e_b, 1.0)
    }

    /// The exact test by plain f64 arithmetic: Pascal's triangle, powers by repeated products,
    /// and the probabilities summed as they are.
    fn plain_p(k_a: usize, n: usize, share: f64) -> f64 {
        let mut row = vec![1.0];
        for _ in 0..n {
            let mut next = vec![1.0; row.len() + 1];
            for i in 1..row.len() {
                next[i] = row[i - 1] + row[i];
            }
            row = next;
        }
        let powers = |base: f64| {
            let mut powers = vec![1.0];
            for i in 0..n {
                powers.push(powers[i] * base);
            }
            powers
        };
        let (share_a, share_b) = (powers(share), powers(1.0 - share));
        let probabilities: Vec<f64> = (0..=n).map(|x| row[x] * share_a[x] * share_b[n - x]).collect();
        let seen = probabilities[k_a] * (1.0 + 1e-7);
        probabilities.iter().filter(|&&p| p <= seen).sum()
    }

    /// erfc by Simpson's rule on 2/√π · e^(−t²), from `x` to where the integrand is below 1e-60.
    fn simpson_erfc(x: f64) -> f64 {
        const STEPS: u32 = 40_000;
        let end = x.max(0.0) + 12.0;
        let h = (end - x) / f64::from(STEPS);
        let f = |i: u32| (-(x + f64::from(i) * h).powi(2)).exp();
        let inner: f64 = (1..STEPS)
            .map(|i| f(i) * if i % 2 == 1 { 4.0 } else { 2.0 })
            .sum();
        (f(0) + inner + f(STEPS)) * h / 3.0 * 2.0 / std::f64::consts::PI.sqrt()
    }

    /// `SplitMix64`, seeded: the same draws on every run.
    struct Draws(u64);

    impl Draws {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// Uniform in [0, 1): 52 random bits under the exponent of 1.
        fn uniform(&mut self) -> f64 {
            f64::from_bits(0x3FF0_0000_0000_0000 | (self.next() >> 12)) - 1.0
        }

        /// Knuth's: uniforms multiplied until the product falls under e^(−mean).
        fn poisson(&mut self, mean: f64) -> u64 {
            let limit = (-mean).exp();
            let mut count = 0;
            let mut product = self.uniform();
            while product > limit {
                count += 1;
                product *= self.uniform();
            }
            count
        }
    }

    #[test]
    fn the_exact_test_matches_the_binomial_sums_worked_by_hand() {
        // n = 12, share 1/2: the weights are C(12, x) over 2¹² = 4096. The 10 seen weighs
        // C(12, 10) = 66; the splits no likelier are 0, 1, 2, 10, 11 and 12:
        // (1 + 12 + 66) · 2 / 4096 = 79/2048.
        assert_close(compared(10, 2, EQUAL).p, 79.0 / 2048.0, 1e-12);
        // n = 40, share 1/2: twice P(X ≤ 10) by symmetry. Σ C(40, x) for x ≤ 10 is
        // 1 221 246 132, so p = 2 · 1 221 246 132 / 2⁴⁰ = 305 311 533 / 137 438 953 472.
        assert_close(
            compared(30, 10, EQUAL).p,
            305_311_533.0 / 137_438_953_472.0,
            1e-12,
        );
        // 5 of 10 is the likeliest split: every split counts.
        assert_close(compared(5, 5, EQUAL).p, 1.0, 1e-12);
        // 100 player-hours now, 200 before: share 1/3, n = 15. The weights are
        // C(15, x) · 2^(15 − x) over 3¹⁵ = 14 348 907: x = 12 weighs 3 640; 13, 14 and 15 weigh
        // 420, 30 and 1; every x ≤ 11 weighs more. p = 4 091 / 14 348 907.
        assert_close(compare(12, 100.0, 3, 200.0, 1.0).p, 4_091.0 / 14_348_907.0, 1e-12);
        // The same windows with 2 now: x = 2 weighs 860 160; no likelier are 0 (32 768),
        // 1 (245 760), 8 (823 680) and 9 to 15 (442 347 together): both tails,
        // p = 2 404 715 / 14 348 907.
        assert_close(
            compare(2, 100.0, 13, 200.0, 1.0).p,
            2_404_715.0 / 14_348_907.0,
            1e-12,
        );
        // Nothing before: only 0 and 25 weigh 1, p = 2 / 2²⁵.
        assert_close(compared(25, 0, EQUAL).p, 1.0 / 16_777_216.0, 1e-12);
    }

    #[test]
    fn the_exact_test_agrees_with_a_plain_summation() {
        for n in [1, 2, 7, 12, 20, 33, 60] {
            for (e_a, e_b) in [(1.0, 1.0), (100.0, 200.0), (3.0, 7.0), (10.0, 1.0), (0.4, 9.0)] {
                for k_a in 0..=n {
                    let k_b = n - k_a;
                    let expected = plain_p(k_a, n, e_a / (e_a + e_b));
                    let actual = compare(k_a as u64, e_a, k_b as u64, e_b, 1.0).p;
                    assert_close(actual, expected, 1e-9);
                }
            }
        }
    }

    #[test]
    fn above_five_thousand_drops_the_normal_approximation_takes_over() {
        // z = ln(2700/2400) / √(1/2700 + 1/2400) = 4.198 413 000 942 075 and
        // p = erfc(z/√2) = 2.687 920 802 491 837 5e-5 (Python's math.erfc), checked here
        // against Simpson's rule too.
        let z = (2_700.0_f64 / 2_400.0).ln() / (1.0 / 2_700.0 + 1.0 / 2_400.0_f64).sqrt();
        let p = compared(2_700, 2_400, EQUAL).p;
        assert_close(p, 2.687_920_802_491_837_5e-5, 2e-7);
        assert_close(p, simpson_erfc(z * FRAC_1_SQRT_2), 2e-7);
    }

    #[test]
    fn the_exact_test_and_the_approximation_agree_where_they_meet() {
        // 2600 against 2400 is the last exact case: p = 0.004 883 6 by exact fractions (Python's
        // `fractions` and `math.comb`), where the approximation would give 0.004 688 8.
        let exact = compared(2_600, 2_400, EQUAL).p;
        assert_close(exact, 0.004_883_649_737_517_518, 1e-7);
        let z = (2_601.0_f64 / 2_400.0).ln() / (1.0 / 2_601.0 + 1.0 / 2_400.0_f64).sqrt();
        let approximated = compared(2_601, 2_400, EQUAL).p;
        assert_close(approximated, simpson_erfc(z * FRAC_1_SQRT_2), 2e-7);
        assert_close(approximated, exact, 0.1);
    }

    #[test]
    fn erfc_stays_within_its_published_error() {
        for x in [-3.0, -1.0, -0.2, 0.0, 0.3, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 9.0] {
            assert_close(erfc(x), simpson_erfc(x), 1.2e-7);
        }
    }

    #[test]
    fn overdispersed_days_widen_the_test() {
        // z = ln 3 / √(2 · (1/30 + 1/10)) = 2.127 453 5, p = erfc(z/√2) = 0.033 382 417
        // (Python's math.erfc): 30 against 10 is rising for steady days, not for spread ones.
        let widened = compare(30, 1.0, 10, 1.0, 2.0);
        assert_close(widened.p, 0.033_382_416_920_992_02, 2e-7);
        assert_eq!(verdict(&widened), Verdict::NoClearChange);
        assert_eq!(verdict(&compared(30, 10, EQUAL)), Verdict::Rising);
        // ln 3 ± 1.96 · √(2 · 2/15).
        assert_close(widened.low.unwrap_or_default(), 1.090_321_537_049_302_7, 1e-12);
    }

    #[test]
    fn a_dispersion_under_one_or_nan_widens_nothing() {
        let steady = compared(30, 10, EQUAL);
        assert_eq!(compare(30, 1.0, 10, 1.0, 0.5), steady);
        assert_eq!(compare(30, 1.0, 10, 1.0, f64::NAN), steady);
    }

    #[test]
    fn the_interval_of_a_known_ratio() {
        // ln 3 ± 1.96 · √(1/30 + 1/10): 3 · e^(∓0.715 690 8).
        let comparison = compared(30, 10, EQUAL);
        assert_eq!(comparison.ratio, Some(3.0));
        assert_close(comparison.low.unwrap_or_default(), 1.466_562_871_316_209_5, 1e-12);
        assert_close(
            comparison.high.unwrap_or_default(),
            6.136_797_934_835_680_5,
            1e-12,
        );
        // Exposure counts: twice the drops over twice the hours is the same rate.
        assert_eq!(compare(60, 2.0, 10, 1.0, 1.0).ratio, Some(3.0));
    }

    #[test]
    fn a_window_without_drops_still_gets_an_interval() {
        // Half a drop added to both: ln(0.5/25.5) ± 1.96 · √(1/0.5 + 1/25.5).
        let falling = compared(0, 25, EQUAL);
        assert_eq!(falling.ratio, Some(0.0));
        assert_close(falling.low.unwrap_or_default(), 0.001_193_664_396_925_495, 1e-12);
        assert_close(falling.high.unwrap_or_default(), 0.322_090_123_057_588, 1e-12);
        assert_eq!(verdict(&falling), Verdict::Falling);

        let rising = compared(25, 0, EQUAL);
        assert_eq!(rising.ratio, Some(f64::INFINITY));
        assert_eq!(verdict(&rising), Verdict::Rising);
    }

    #[test]
    fn no_drops_at_all_is_no_test() {
        let comparison = compared(0, 0, EQUAL);
        assert_eq!(
            comparison,
            Comparison {
                events: 0,
                ratio: None,
                low: None,
                high: None,
                p: 1.0
            }
        );
        assert_eq!(verdict(&comparison), Verdict::NotEnoughDrops);
    }

    #[test]
    fn an_unusable_exposure_gives_not_enough_drops() {
        for bad in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for comparison in [compare(50, bad, 10, 1.0, 1.0), compare(50, 1.0, 10, bad, 1.0)] {
                assert_eq!(
                    comparison,
                    Comparison {
                        events: 60,
                        ratio: None,
                        low: None,
                        high: None,
                        p: 1.0
                    }
                );
                assert_eq!(verdict(&comparison), Verdict::NotEnoughDrops);
                assert_eq!(verdict_in_list(&comparison, true), Verdict::NotEnoughDrops);
            }
        }
    }

    #[test]
    fn extreme_exposures_never_give_a_nan() {
        let tiny = f64::from_bits(1);
        let pairs = [
            (tiny, f64::MAX),
            (f64::MAX, tiny),
            (f64::MIN_POSITIVE, 1e300),
            (tiny, tiny),
        ];
        for (e_a, e_b) in pairs {
            for (k_a, k_b) in [(0, 30), (30, 0), (30, 10), (6_000, 10)] {
                let comparison = compare(k_a, e_a, k_b, e_b, 1.0);
                assert!(
                    comparison.ratio.is_some_and(|ratio| !ratio.is_nan()),
                    "{comparison:?}"
                );
                assert!((0.0..=1.0).contains(&comparison.p), "{comparison:?}");
                assert!(comparison.low.is_some_and(|low| !low.is_nan()), "{comparison:?}");
                assert!(
                    comparison.high.is_some_and(|high| !high.is_nan()),
                    "{comparison:?}"
                );
                assert!(!compare(k_a, e_a, k_b, e_b, 3.0).p.is_nan());
            }
        }
        // Both rates overflow: the ratio still comes through.
        assert_close(
            compare(30, tiny, 10, tiny, 1.0).ratio.unwrap_or_default(),
            3.0,
            1e-12,
        );
    }

    #[test]
    fn nineteen_drops_are_never_rising_or_falling() {
        for (e_a, e_b) in [(1.0, 1.0), (1.0, 10.0), (10.0, 1.0)] {
            for k_a in 0..=19 {
                let comparison = compare(k_a, e_a, 19 - k_a, e_b, 1.0);
                assert_eq!(verdict(&comparison), Verdict::NotEnoughDrops);
                assert_eq!(verdict_in_list(&comparison, true), Verdict::NotEnoughDrops);
            }
        }
        // p = 2 / 2²⁰.
        assert_eq!(verdict(&compared(20, 0, EQUAL)), Verdict::Rising);
    }

    #[test]
    fn a_fourteen_percent_rise_with_a_tiny_p_is_no_clear_change() {
        // z = 30.2, p ≈ 6.6e-201.
        let rise = compared(114_000, 100_000, EQUAL);
        assert!(rise.p < 1e-100);
        assert_eq!(verdict(&rise), Verdict::NoClearChange);
        assert_eq!(verdict(&compared(116_000, 100_000, EQUAL)), Verdict::Rising);
        assert_eq!(verdict(&compared(88_000, 100_000, EQUAL)), Verdict::NoClearChange);
        assert_eq!(verdict(&compared(86_000, 100_000, EQUAL)), Verdict::Falling);
    }

    #[test]
    fn a_change_needs_p_under_one_percent() {
        // Tripled, 20 drops, but p = 2 · Σ C(20, x) for x ≤ 5 / 2²⁰ = 5 425/131 072 ≈ 0.041.
        let comparison = compared(15, 5, EQUAL);
        assert_close(comparison.p, 5_425.0 / 131_072.0, 1e-12);
        assert_eq!(verdict(&comparison), Verdict::NoClearChange);
    }

    #[test]
    fn in_a_list_the_false_discovery_control_decides() {
        let weak = compared(15, 5, EQUAL);
        assert_eq!(verdict_in_list(&weak, true), Verdict::Rising);
        assert_eq!(
            verdict_in_list(&compared(30, 10, EQUAL), false),
            Verdict::NoClearChange
        );
        assert_eq!(verdict_in_list(&compared(5, 15, EQUAL), true), Verdict::Falling);
        let small = compared(114_000, 100_000, EQUAL);
        assert_eq!(verdict_in_list(&small, true), Verdict::NoClearChange);
    }

    #[test]
    fn a_list_holds_only_its_tested_rows_to_the_false_discovery_rate() {
        let strong = compared(30, 10, EQUAL); // p ≈ 0.0022
        let weak = compared(15, 5, EQUAL); // p ≈ 0.041
        let few = compared(8, 1, EQUAL); // 9 drops: outside the control
        let unusable = compare(40, 0.0, 10, 1.0, 1.0); // no exposure: outside too
        let steady = compared(20, 20, EQUAL); // p = 1
        // Two tested rows: 0.0022 ≤ 0.025 and 0.041 ≤ 0.05, both fall.
        assert_eq!(
            list_verdicts(&[strong, few, weak, unusable]),
            [
                Verdict::Rising,
                Verdict::NotEnoughDrops,
                Verdict::Rising,
                Verdict::NotEnoughDrops
            ]
        );
        // Three: 0.0022 ≤ 0.0167 still, 0.041 > 0.0333 no longer.
        assert_eq!(
            list_verdicts(&[strong, weak, steady]),
            [Verdict::Rising, Verdict::NoClearChange, Verdict::NoClearChange]
        );
        assert_eq!(list_verdicts(&[]), Vec::<Verdict>::new());
    }

    #[test]
    fn benjamini_hochberg_rejects_up_to_the_largest_p_under_its_share() {
        // Sorted: 0.005, 0.01, 0.03, 0.04, 0.20 against 0.01, 0.02, 0.03, 0.04, 0.05. The
        // largest under its share is 0.04, fourth: the four smallest fall.
        assert_eq!(
            benjamini_hochberg(&[0.01, 0.04, 0.03, 0.005, 0.20], 0.05),
            [true, true, true, true, false]
        );
        // 0.02 is over its own share (0.05/3) but 0.025 is under its (0.10/3): both fall.
        assert_eq!(benjamini_hochberg(&[0.025, 0.9, 0.02], 0.05), [true, false, true]);
        assert_eq!(benjamini_hochberg(&[0.02, 0.3, 0.6], 0.05), [false, false, false]);
    }

    #[test]
    fn benjamini_hochberg_of_nothing_is_nothing() {
        assert_eq!(benjamini_hochberg(&[], 0.05), Vec::<bool>::new());
    }

    #[test]
    fn tied_p_values_fall_together() {
        // At rank 3, 0.03 ≤ 3 · 0.05 / 4 = 0.0375: all three, wherever they stand.
        assert_eq!(
            benjamini_hochberg(&[0.03, 0.5, 0.03, 0.03], 0.05),
            [true, false, true, true]
        );
        // Alone 0.04 is over 0.025; tied, the second is under 0.05.
        assert_eq!(benjamini_hochberg(&[0.04, 0.04], 0.05), [true, true]);
        assert_eq!(
            benjamini_hochberg(&[0.04, 0.04, 0.9], 0.05),
            [false, false, false]
        );
    }

    #[test]
    fn a_nan_p_value_is_never_rejected_and_helps_no_other() {
        // Counted as 1, last: 0.03 stays first and over 0.025.
        let negative_nan = f64::from_bits(0xFFF8_0000_0000_0000);
        assert_eq!(benjamini_hochberg(&[f64::NAN, 0.03], 0.05), [false, false]);
        assert_eq!(benjamini_hochberg(&[negative_nan, 0.03], 0.05), [false, false]);
        assert_eq!(benjamini_hochberg(&[f64::NAN, 0.001], 0.05), [false, true]);
    }

    #[test]
    fn days_in_proportion_to_their_exposure_have_no_overdispersion() {
        assert_close(dispersion(&[(50, 100.0), (25, 50.0), (100, 200.0)]), 1.0, 1e-12);
    }

    #[test]
    fn poisson_days_have_a_dispersion_near_one() {
        let mut draws = Draws(0x0b5e_7e47);
        let trials = 200;
        let mut total = 0.0;
        for _ in 0..trials {
            let days: Vec<(u64, f64)> = (0..28_u32)
                .map(|day| {
                    let exposure = 80.0 + 10.0 * f64::from(day % 5);
                    (draws.poisson(0.5 * exposure), exposure)
                })
                .collect();
            let phi = dispersion(&days);
            assert!(phi >= 1.0);
            total += phi;
        }
        // max(1, χ²₂₇ / 27) averages about 1.11.
        let mean = total / f64::from(trials);
        assert!((1.0..1.25).contains(&mean), "mean dispersion {mean}");
    }

    #[test]
    fn overdispersed_days_have_a_dispersion_well_above_one() {
        // 350 drops over 700 hours: 50 expected a day. The squared gaps add up to 11 858,
        // χ² = 11 858 / 50 = 237.16, over 6 degrees: 39.526 666…
        let days = [5, 95, 10, 90, 50, 2, 98].map(|drops| (drops, 100.0));
        assert_close(dispersion(&days), 237.16 / 6.0, 1e-12);
    }

    #[test]
    fn fewer_than_two_usable_days_or_no_drops_give_one() {
        assert_eq!(dispersion(&[]), 1.0);
        assert_eq!(dispersion(&[(10, 5.0)]), 1.0);
        assert_eq!(dispersion(&[(10, 5.0), (3, 0.0)]), 1.0);
        assert_eq!(dispersion(&[(10, 5.0), (3, f64::NAN)]), 1.0);
        assert_eq!(dispersion(&[(10, 5.0), (3, f64::INFINITY)]), 1.0);
        assert_eq!(dispersion(&[(0, 5.0), (0, 8.0)]), 1.0);
    }

    #[test]
    fn a_constant_rate_is_called_rising_in_at_most_one_week_in_a_hundred() {
        let mut draws = Draws(0x000d_20b5);
        let pairs = 2_000;
        // 60 drops a week expected at equal exposure, then the same rate over 700 hours
        // against 1000.
        for (e_a, e_b, mean_a, mean_b) in [(1_000.0, 1_000.0, 60.0, 60.0), (700.0, 1_000.0, 42.0, 60.0)] {
            let (mut rising, mut falling) = (0, 0);
            for _ in 0..pairs {
                let now = draws.poisson(mean_a);
                let before = draws.poisson(mean_b);
                match verdict(&compare(now, e_a, before, e_b, 1.0)) {
                    Verdict::Rising => rising += 1,
                    Verdict::Falling => falling += 1,
                    _ => {}
                }
            }
            assert!(rising <= pairs / 100, "{rising} weeks of {pairs} called rising");
            assert!(
                falling <= pairs / 100,
                "{falling} weeks of {pairs} called falling"
            );
        }
    }

    #[test]
    fn a_sixty_percent_rise_from_two_hundred_drops_a_week_is_seen_most_weeks() {
        let mut draws = Draws(0x051c_77e5);
        let pairs = 2_000;
        let seen = (0..pairs)
            .filter(|_| {
                let now = draws.poisson(320.0);
                let before = draws.poisson(200.0);
                verdict(&compared(now, before, EQUAL)) == Verdict::Rising
            })
            .count();
        assert!(seen * 100 > pairs * 80, "{seen} weeks of {pairs} called rising");
    }

    #[test]
    fn drops_per_hundred_hours_need_play() {
        // 3 drops in an hour and a half.
        assert_eq!(per_hundred_hours(3, 5_400_000), Some(200.0));
        assert_eq!(per_hundred_hours(0, 3_600_000), Some(0.0));
        assert_eq!(per_hundred_hours(3, 0), None);
        assert_eq!(per_hundred_hours(3, -5), None);
    }
}
