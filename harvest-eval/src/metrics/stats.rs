use serde::{Deserialize, Serialize};

pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    pub count: usize,
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    pub max: f64,
}

impl Latency {
    pub fn of(values: &[f64]) -> Self {
        Self {
            count: values.len(),
            p50: percentile(values, 50.0),
            p90: percentile(values, 90.0),
            p99: percentile(values, 99.0),
            max: values.iter().copied().fold(0.0, f64::max),
        }
    }
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

pub fn bootstrap_mean_ci(values: &[f64], resamples: usize, seed: u64) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mut rng = XorShift(seed | 1);
    let mut means: Vec<f64> = (0..resamples)
        .map(|_| {
            let sum: f64 = (0..values.len())
                .map(|_| values[(rng.next() % values.len() as u64) as usize])
                .sum();
            sum / values.len() as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    (percentile(&means, 2.5), percentile(&means, 97.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&values, 50.0), 50.0);
        assert_eq!(percentile(&values, 99.0), 99.0);
        assert_eq!(percentile(&[7.0], 99.0), 7.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
        let latency = Latency::of(&values);
        assert_eq!(
            (latency.p90, latency.max, latency.count),
            (90.0, 100.0, 100)
        );
    }

    #[test]
    fn bootstrap_intervals_bracket_the_mean_and_shrink_with_more_data() {
        let small: Vec<f64> = (0..20)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let large: Vec<f64> = (0..2000)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let (lo, hi) = bootstrap_mean_ci(&small, 2000, 7);
        assert!(lo < 0.5 && hi > 0.5);
        let (lo2, hi2) = bootstrap_mean_ci(&large, 2000, 7);
        assert!(hi2 - lo2 < hi - lo);
        assert_eq!(
            bootstrap_mean_ci(&small, 500, 3),
            bootstrap_mean_ci(&small, 500, 3)
        );
    }
}
