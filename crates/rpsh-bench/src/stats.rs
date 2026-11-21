//! Summary statistics over timed samples and their resource usage.

use std::time::Duration;

/// One timed run: wall clock, CPU time and the process's peak RSS.
#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub(crate) elapsed: Duration,
    pub(crate) user: Duration,
    pub(crate) system: Duration,
    pub(crate) max_rss_kib: u64,
}

pub(crate) struct Summary {
    pub(crate) samples_ms: Vec<f64>,
    pub(crate) min_ms: f64,
    pub(crate) median_ms: f64,
    pub(crate) mean_ms: f64,
    /// Sample standard deviation; 0 for a single sample.
    pub(crate) stddev_ms: f64,
    pub(crate) max_ms: f64,
    /// 95% bootstrap confidence interval for the median.
    pub(crate) median_ci95_ms: [f64; 2],
    /// Samples outside the Tukey fences (1.5 × IQR), if there are enough
    /// samples to define them.
    pub(crate) outliers_ms: Vec<f64>,
    pub(crate) mean_user_ms: f64,
    pub(crate) mean_system_ms: f64,
    /// The largest peak RSS any run reported, in KiB.
    pub(crate) max_rss_kib: u64,
}

/// Summarizes at least one sample.
pub(crate) fn summarize(samples: &[Sample]) -> Summary {
    let samples_ms: Vec<f64> = samples
        .iter()
        .map(|sample| sample.elapsed.as_secs_f64() * 1000.0)
        .collect();
    let mut sorted = samples_ms.clone();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let median_ms = median_sorted(&sorted);
    let mean_ms = sorted.iter().sum::<f64>() / count as f64;
    let stddev_ms = if count > 1 {
        let squares: f64 = sorted.iter().map(|ms| (ms - mean_ms).powi(2)).sum();
        (squares / (count - 1) as f64).sqrt()
    } else {
        0.0
    };
    let mean_user_ms =
        samples.iter().map(|s| s.user.as_secs_f64()).sum::<f64>() * 1000.0 / count as f64;
    let mean_system_ms =
        samples.iter().map(|s| s.system.as_secs_f64()).sum::<f64>() * 1000.0 / count as f64;
    Summary {
        min_ms: sorted[0],
        max_ms: sorted[count - 1],
        median_ms,
        mean_ms,
        stddev_ms,
        median_ci95_ms: bootstrap_median_ci(&sorted),
        outliers_ms: outliers(&sorted),
        mean_user_ms,
        mean_system_ms,
        max_rss_kib: samples
            .iter()
            .map(|sample| sample.max_rss_kib)
            .max()
            .unwrap_or(0),
        samples_ms,
    }
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let count = sorted.len();
    if count % 2 == 1 {
        sorted[count / 2]
    } else {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let rank = fraction * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        sorted[low]
    } else {
        sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64)
    }
}

/// The 2.5th and 97.5th percentiles of the median over resampled runs.
///
/// The generator is fixed, so the same samples always give the same interval
/// and a result file does not change when it is regenerated.
fn bootstrap_median_ci(sorted: &[f64]) -> [f64; 2] {
    const RESAMPLES: usize = 10_000;
    if sorted.len() < 2 {
        return [sorted[0], sorted[0]];
    }
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut scratch = vec![0.0; sorted.len()];
    let mut medians = Vec::with_capacity(RESAMPLES);
    for _ in 0..RESAMPLES {
        for slot in &mut scratch {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *slot = sorted[((state >> 33) as usize) % sorted.len()];
        }
        scratch.sort_by(f64::total_cmp);
        medians.push(median_sorted(&scratch));
    }
    medians.sort_by(f64::total_cmp);
    [percentile(&medians, 0.025), percentile(&medians, 0.975)]
}

/// Tukey's fences. Fewer than four samples cannot define a quartile split,
/// so nothing is called an outlier then.
fn outliers(sorted: &[f64]) -> Vec<f64> {
    if sorted.len() < 4 {
        return Vec::new();
    }
    let q1 = percentile(sorted, 0.25);
    let q3 = percentile(sorted, 0.75);
    let iqr = q3 - q1;
    let low = q1 - 1.5 * iqr;
    let high = q3 + 1.5 * iqr;
    sorted
        .iter()
        .copied()
        .filter(|value| *value < low || *value > high)
        .collect()
}

/// The geometric mean, the right average for ratios such as speedups.
pub(crate) fn geometric_mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let log_sum: f64 = values.iter().map(|value| value.ln()).sum();
    Some((log_sum / values.len() as f64).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(millis: u64) -> Sample {
        Sample {
            elapsed: Duration::from_millis(millis),
            user: Duration::from_micros(millis * 500),
            system: Duration::from_micros(millis * 100),
            max_rss_kib: millis * 10,
        }
    }

    fn samples(values: &[u64]) -> Vec<Sample> {
        values.iter().map(|value| sample(*value)).collect()
    }

    #[test]
    fn median_of_odd_and_even_counts() {
        assert_eq!(summarize(&samples(&[5, 1, 3])).median_ms, 3.0);
        assert_eq!(summarize(&samples(&[4, 1, 3, 2])).median_ms, 2.5);
    }

    #[test]
    fn spread_and_resources() {
        let summary = summarize(&samples(&[2, 4, 4, 4, 5, 5, 7, 9]));
        assert_eq!(summary.min_ms, 2.0);
        assert_eq!(summary.max_ms, 9.0);
        assert_eq!(summary.mean_ms, 5.0);
        assert!((summary.stddev_ms - 2.138).abs() < 0.001);
        assert_eq!(summary.mean_user_ms, 2.5);
        assert_eq!(summary.mean_system_ms, 0.5);
        assert_eq!(summary.max_rss_kib, 90);
        assert_eq!(summarize(&samples(&[7])).stddev_ms, 0.0);
    }

    #[test]
    fn confidence_interval_brackets_the_median() {
        let summary = summarize(&samples(&[10, 11, 12, 13, 14, 15, 16, 17, 18, 19]));
        assert!(summary.median_ci95_ms[0] <= summary.median_ms);
        assert!(summary.median_ci95_ms[1] >= summary.median_ms);
        // A fixed generator means a fixed interval.
        let again = summarize(&samples(&[10, 11, 12, 13, 14, 15, 16, 17, 18, 19]));
        assert_eq!(summary.median_ci95_ms, again.median_ci95_ms);
    }

    #[test]
    fn finds_outliers_outside_the_tukey_fences() {
        let summary = summarize(&samples(&[10, 10, 11, 11, 12, 12, 13, 100]));
        assert_eq!(summary.outliers_ms, vec![100.0]);
        assert!(summarize(&samples(&[1, 2, 3])).outliers_ms.is_empty());
    }

    #[test]
    fn geometric_mean_of_ratios() {
        assert_eq!(geometric_mean(&[]), None);
        let mean = geometric_mean(&[2.0, 8.0]).unwrap();
        assert!((mean - 4.0).abs() < 1e-9);
    }
}
