//! Summary statistics over wall-clock samples.

use std::time::Duration;

pub(crate) struct Summary {
    pub(crate) samples_ms: Vec<f64>,
    pub(crate) min_ms: f64,
    pub(crate) median_ms: f64,
    pub(crate) mean_ms: f64,
    /// Sample standard deviation; 0 for a single sample.
    pub(crate) stddev_ms: f64,
    pub(crate) max_ms: f64,
}

/// Summarizes at least one sample.
pub(crate) fn summarize(samples: &[Duration]) -> Summary {
    let samples_ms: Vec<f64> = samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1000.0)
        .collect();
    let mut sorted = samples_ms.clone();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let median_ms = if count % 2 == 1 {
        sorted[count / 2]
    } else {
        (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
    };
    let mean_ms = sorted.iter().sum::<f64>() / count as f64;
    let stddev_ms = if count > 1 {
        let squares: f64 = sorted.iter().map(|ms| (ms - mean_ms).powi(2)).sum();
        (squares / (count - 1) as f64).sqrt()
    } else {
        0.0
    };
    Summary {
        min_ms: sorted[0],
        max_ms: sorted[count - 1],
        median_ms,
        mean_ms,
        stddev_ms,
        samples_ms,
    }
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

    fn millis(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn median_of_odd_and_even_counts() {
        assert_eq!(summarize(&millis(&[5, 1, 3])).median_ms, 3.0);
        assert_eq!(summarize(&millis(&[4, 1, 3, 2])).median_ms, 2.5);
    }

    #[test]
    fn spread() {
        let summary = summarize(&millis(&[2, 4, 4, 4, 5, 5, 7, 9]));
        assert_eq!(summary.min_ms, 2.0);
        assert_eq!(summary.max_ms, 9.0);
        assert_eq!(summary.mean_ms, 5.0);
        assert!((summary.stddev_ms - 2.138).abs() < 0.001);
        assert_eq!(summarize(&millis(&[7])).stddev_ms, 0.0);
    }

    #[test]
    fn geometric_mean_of_ratios() {
        assert_eq!(geometric_mean(&[]), None);
        let mean = geometric_mean(&[2.0, 8.0]).unwrap();
        assert!((mean - 4.0).abs() < 1e-9);
    }
}
