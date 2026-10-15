use hdrhistogram::Histogram;
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
    pub fn new() -> Self {
        Self {
            min: u64::MAX,
            max: 0,
            mean: 0.0,
            percentiles: HashMap::new(),
            std_dev: 0.0,
            count: 0,
        }
    }

    pub fn record(&mut self, value: u64) {
        // Simple recording - for full histogram use hdrhistogram
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        // Update mean incrementally
        self.count += 1;
        let delta = value as f64 - self.mean;
        self.mean += delta / self.count as f64;
    }

    #[allow(dead_code)] // kept for histogram-based reports; not used by the CLI yet
    pub fn from_histogram(hist: &Histogram<u64>) -> Self {
        let mut percentiles = HashMap::new();
        for &p in &[50, 90, 95, 99] {
            percentiles.insert(p, hist.value_at_percentile(p as f64));
        }
        // 99.9 percentile
        percentiles.insert(99, hist.value_at_percentile(99.9));

        let count = hist.len();
        let mean = hist.mean();

        let mut sum_sq = 0.0;
        for v in hist.iter_recorded() {
            let diff = v.value_iterated_to() as f64 - mean;
            sum_sq += diff * diff * v.count_at_value() as f64;
        }
        let std_dev = if count > 0 {
            (sum_sq / count as f64).sqrt()
        } else {
            0.0
        };

        Self {
            min: hist.min(),
            max: hist.max(),
            mean,
            percentiles,
            std_dev,
            count,
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
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
