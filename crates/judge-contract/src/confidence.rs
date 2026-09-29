//! The `confidence` of Choice and Score answers, as TypeSafe defines it
//! (decider-ai 1.5.0 `choice_confidence` / `score_confidence`), so a threshold
//! means the same whichever provider answered. Both read the probabilities in
//! option (level) order; a distribution summing to 0 counts as uniform.

fn normalized(p: &[f64]) -> Vec<f64> {
    let total: f64 = p.iter().sum();
    if total == 0.0 {
        vec![1.0 / p.len() as f64; p.len()]
    } else {
        p.iter().map(|x| x / total).collect()
    }
}

/// The first of equal maxima, as Python's `max()` picks it.
fn likeliest(p: &[f64]) -> usize {
    (0..p.len()).fold(0, |best, i| if p[i] > p[best] { i } else { best })
}

/// Choice: `(n·p_max − 1) / (n − 1)`, 0 for a uniform distribution and 1 for
/// all mass on one option; 1 for a single option.
pub fn choice(p: &[f64]) -> f64 {
    let n = p.len();
    if n <= 1 {
        return 1.0;
    }
    let p = normalized(p);
    ((n as f64 * p[likeliest(&p)] - 1.0) / (n as f64 - 1.0)).clamp(0.0, 1.0)
}

/// Score: 1 − the expected distance from the likeliest level over D, the mean
/// distance of the levels from the middle of the scale; 1 for a single level.
/// With two levels it equals `choice`.
pub fn score(p: &[f64]) -> f64 {
    let n = p.len();
    if n <= 1 {
        return 1.0;
    }
    let p = normalized(p);
    let best = likeliest(&p);
    let spread: f64 = p
        .iter()
        .enumerate()
        .map(|(i, x)| x * i.abs_diff(best) as f64)
        .sum();
    let middle = (n as f64 - 1.0) / 2.0;
    let uniform = (0..n).map(|i| (i as f64 - middle).abs()).sum::<f64>() / n as f64;
    (1.0 - spread / uniform).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{choice, score};

    /// Values from decider-ai 1.5.0's `choice_confidence` / `score_confidence`.
    #[test]
    fn matches_the_reference_definitions() {
        for (p, c, s) in [
            (&[0.9, 0.06, 0.04][..], 0.85, 0.79),
            (&[0.25, 0.25, 0.25, 0.25], 0.0, 0.0),
            (&[1.0], 1.0, 1.0),
            (&[0.5, 0.5], 0.0, 0.0),
            (&[0.0, 0.0, 0.0], 0.0, 0.0),
            (&[0.2, 0.7, 0.1], 0.55, 0.55),
        ] {
            assert!((choice(p) - c).abs() < 1e-9, "choice {p:?}");
            assert!((score(p) - s).abs() < 1e-9, "score {p:?}");
        }
    }
}
