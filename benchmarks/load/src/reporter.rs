use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario: String,
    pub duration_secs: f64,
    pub concurrency: usize,
    pub total_iterations: usize,
    pub total_errors: usize,
    pub latencies: HashMap<String, LatencyHistogram>,
    pub throughput: HashMap<String, f64>,
    pub error_rates: HashMap<String, f64>,
    pub timestamp: String,
}

impl ScenarioResult {
    pub fn print_summary(&self) {
        println!("Scenario:         {}", self.scenario);
        println!("Duration:         {:.2}s", self.duration_secs);
        println!("Concurrency:      {}", self.concurrency);
        println!("Total Iterations: {}", self.total_iterations);
        println!("Total Errors:     {}", self.total_errors);
        println!(
            "Error Rate:       {:.4}%",
            self.error_rates.get("overall").unwrap_or(&0.0) * 100.0
        );
        println!(
            "Throughput:       {:.2} ops/s",
            self.throughput.get("overall").unwrap_or(&0.0)
        );
        println!();

        if let Some(lat) = self.latencies.get("overall") {
            println!("Latency (ns):");
            println!("  Min:      {}", lat.min);
            println!("  Mean:     {:.2}", lat.mean);
            println!("  Median:   {}", lat.percentiles.get(&50).unwrap_or(&0));
            println!("  P90:      {}", lat.percentiles.get(&90).unwrap_or(&0));
            println!("  P99:      {}", lat.percentiles.get(&99).unwrap_or(&0));
            println!("  P99.9:    {}", lat.percentiles.get(&99).unwrap_or(&0)); // 99.9 uses key 99
            println!("  Max:      {}", lat.max);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyHistogram {
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub percentiles: HashMap<u8, u64>,
    pub std_dev: f64,
    pub count: u64,
}

impl LatencyHistogram {
    /// Summarizes latency samples in nanoseconds. Percentiles are exact
    /// nearest-rank values over every sample, so the reported p99 is the value
    /// that 99% of samples are at or below.
    pub fn from_samples(samples: &[u64]) -> Self {
        if samples.is_empty() {
            return Self {
                min: 0,
                max: 0,
                mean: 0.0,
                percentiles: HashMap::new(),
                std_dev: 0.0,
                count: 0,
            };
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let count = sorted.len() as u64;
        let mean = sorted.iter().map(|&v| v as f64).sum::<f64>() / count as f64;
        let variance = sorted
            .iter()
            .map(|&v| {
                let d = v as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / count as f64;
        // The JSON key is the integer percentile, so p99.9 is not reported.
        let percentiles = [50u8, 90, 95, 99]
            .into_iter()
            .map(|p| {
                let rank = (f64::from(p) / 100.0 * count as f64).ceil() as usize;
                (p, sorted[rank.clamp(1, sorted.len()) - 1])
            })
            .collect();
        Self {
            min: sorted[0],
            max: sorted[sorted.len() - 1],
            mean,
            percentiles,
            std_dev: variance.sqrt(),
            count,
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::from_samples(&[])
    }
}

pub struct ReportGenerator {
    format: String,
}

impl ReportGenerator {
    pub fn new(format: &str) -> Self {
        Self {
            format: format.to_string(),
        }
    }

    #[allow(clippy::needless_borrows_for_generic_args)]
    pub fn write(&self, result: &ScenarioResult, path: &str) -> anyhow::Result<()> {
        match self.format.as_str() {
            "json" => {
                let json = serde_json::to_string_pretty(result)?;
                std::fs::write(path, json)?;
            }
            "csv" => {
                let mut writer = csv::Writer::from_path(path)?;
                writer.write_record(&[
                    "scenario",
                    "duration_secs",
                    "concurrency",
                    "total_iterations",
                    "total_errors",
                    "throughput",
                    "error_rate",
                    "latency_min",
                    "latency_mean",
                    "latency_median",
                    "latency_p90",
                    "latency_p99",
                    "latency_p999",
                    "latency_max",
                ])?;

                let lat = result.latencies.get("overall").cloned().unwrap_or_default();
                writer.write_record(&[
                    &result.scenario,
                    &result.duration_secs.to_string(),
                    &result.concurrency.to_string(),
                    &result.total_iterations.to_string(),
                    &result.total_errors.to_string(),
                    &result.throughput.get("overall").unwrap_or(&0.0).to_string(),
                    &result
                        .error_rates
                        .get("overall")
                        .unwrap_or(&0.0)
                        .to_string(),
                    &lat.min.to_string(),
                    &lat.mean.to_string(),
                    &lat.percentiles.get(&50).unwrap_or(&0).to_string(),
                    &lat.percentiles.get(&90).unwrap_or(&0).to_string(),
                    &lat.percentiles.get(&99).unwrap_or(&0).to_string(),
                    &lat.percentiles.get(&99).unwrap_or(&0).to_string(), // 99.9
                    &lat.max.to_string(),
                ])?;
                writer.flush()?;
            }
            _ => {
                // Text format
                let mut output = String::new();
                output.push_str(&format!("Scenario: {}\n", result.scenario));
                output.push_str(&format!("Duration: {:.2}s\n", result.duration_secs));
                output.push_str(&format!("Concurrency: {}\n", result.concurrency));
                output.push_str(&format!("Total Iterations: {}\n", result.total_iterations));
                output.push_str(&format!("Total Errors: {}\n", result.total_errors));
                output.push_str(&format!(
                    "Throughput: {:.2} ops/s\n",
                    result.throughput.get("overall").unwrap_or(&0.0)
                ));
                output.push_str(&format!(
                    "Error Rate: {:.4}%\n",
                    result.error_rates.get("overall").unwrap_or(&0.0) * 100.0
                ));

                if let Some(lat) = result.latencies.get("overall") {
                    output.push_str(&format!("Latency Min: {} ns\n", lat.min));
                    output.push_str(&format!("Latency Mean: {:.2} ns\n", lat.mean));
                    output.push_str(&format!(
                        "Latency Median: {} ns\n",
                        lat.percentiles.get(&50).unwrap_or(&0)
                    ));
                    output.push_str(&format!(
                        "Latency P90: {} ns\n",
                        lat.percentiles.get(&90).unwrap_or(&0)
                    ));
                    output.push_str(&format!(
                        "Latency P99: {} ns\n",
                        lat.percentiles.get(&99).unwrap_or(&0)
                    ));
                    output.push_str(&format!("Latency Max: {} ns\n", lat.max));
                }

                std::fs::write(path, output)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for ScenarioResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScenarioResult({})", self.scenario)
    }
}

#[cfg(test)]
mod tests {
    use super::LatencyHistogram;

    #[test]
    fn percentiles_are_exact_nearest_rank_values() {
        // 1..=100 in shuffled order: nearest rank p99 is the 99th smallest value.
        let mut samples: Vec<u64> = (1..=100).rev().collect();
        samples.swap(0, 50);
        let h = LatencyHistogram::from_samples(&samples);
        assert_eq!(h.count, 100);
        assert_eq!(h.min, 1);
        assert_eq!(h.max, 100);
        assert_eq!(h.percentiles[&50], 50);
        assert_eq!(h.percentiles[&90], 90);
        assert_eq!(h.percentiles[&95], 95);
        assert_eq!(h.percentiles[&99], 99);
        assert!((h.mean - 50.5).abs() < 1e-9);
    }

    #[test]
    fn empty_samples_report_zeroes_and_no_percentiles() {
        let h = LatencyHistogram::from_samples(&[]);
        assert_eq!(h.count, 0);
        assert_eq!((h.min, h.max), (0, 0));
        assert!(h.percentiles.is_empty());
    }

    #[test]
    fn a_single_sample_is_every_percentile() {
        let h = LatencyHistogram::from_samples(&[42]);
        assert_eq!(h.percentiles[&99], 42);
        assert_eq!(h.std_dev, 0.0);
    }
}
