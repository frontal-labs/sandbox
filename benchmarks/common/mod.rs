use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, Throughput};
use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

pub mod vfs_helpers;
pub mod cas_helpers;
pub mod shell_helpers;
pub mod runtime_helpers;
pub mod wasm_helpers;
pub mod session_helpers;
pub mod promotion_helpers;

pub use vfs_helpers::*;
pub use cas_helpers::*;
pub use shell_helpers::*;
pub use runtime_helpers::*;
pub use wasm_helpers::*;
pub use session_helpers::*;
pub use promotion_helpers::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub target: Option<String>,
    pub unit: String,
    pub samples: usize,
    pub mean: f64,
    pub median: f64,
    pub p90: f64,
    pub p99: f64,
    pub p999: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub throughput: Option<f64>,
    pub timestamp: String,
}

impl BenchmarkResult {
    pub fn from_criterion(group: &BenchmarkGroup<'_, WallTime>, id: &BenchmarkId) -> Vec<Self> {
        let mut results = Vec::new();
        
        for (name, summary) in group.bench_functions() {
            if let Some(measurement) = group.bench_function(name) {
                let stats = measurement.mean();
                let result = BenchmarkResult {
                    name: format!("{:?}", id),
                    target: None,
                    unit: "ns".to_string(),
                    samples: measurement.sample_count() as usize,
                    mean: stats.as_secs_f64() * 1_000_000_000.0,
                    median: measurement.median().as_secs_f64() * 1_000_000_000.0,
                    p90: 0.0,
                    p99: 0.0,
                    p999: 0.0,
                    std_dev: measurement.std_dev().as_secs_f64() * 1_000_000_000.0,
                    min: measurement.min().as_secs_f64() * 1_000_000_000.0,
                    max: measurement.max().as_secs_f64() * 1_000_000_000.0,
                    throughput: None,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                results.push(result);
            }
        }
        results
    }

    pub fn from_histogram(name: &str, histogram: &Histogram<u64>, unit: &str, target: Option<&str>) -> Self {
        let total_count = histogram.len() as f64;
        let mean = histogram.mean() as f64;
        let median = histogram.value_at_percentile(50.0) as f64;
        let p90 = histogram.value_at_percentile(90.0) as f64;
        let p99 = histogram.value_at_percentile(99.0) as f64;
        let p999 = histogram.value_at_percentile(99.9) as f64;
        let min = histogram.min() as f64;
        let max = histogram.max() as f64;
        
        let mut sum_sq = 0.0;
        for v in histogram.iter_recorded() {
            let diff = v.value_iterated_to() as f64 - mean;
            sum_sq += diff * diff * v.count_at_value() as f64;
        }
        let std_dev = (sum_sq / total_count).sqrt();

        Self {
            name: name.to_string(),
            target: target.map(|s| s.to_string()),
            unit: unit.to_string(),
            samples: histogram.len() as usize,
            mean,
            median,
            p90,
            p99,
            p999,
            std_dev,
            min,
            max,
            throughput: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn passes_target(&self) -> Option<bool> {
        self.target.as_ref().and_then(|t| {
            parse_target(t).ok().map(|target_val| {
                match self.unit.as_str() {
                    "ns" | "µs" | "ms" => self.p99 <= target_val,
                    "ops/s" | "MB/s" => self.mean >= target_val,
                    _ => false,
                }
            })
        })
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap()
    }
}

fn parse_target(target: &str) -> Result<f64, String> {
    let target = target.trim();
    let (num_str, multiplier) = if target.ends_with("ms") {
        (target.trim_end_matches("ms"), 1_000_000.0)
    } else if target.ends_with("µs") || target.ends_with("us") {
        (target.trim_end_matches("µs").trim_end_matches("us"), 1_000.0)
    } else if target.ends_with("ns") {
        (target.trim_end_matches("ns"), 1.0)
    } else if target.ends_with("s") {
        (target.trim_end_matches("s"), 1_000_000_000.0)
    } else if target.ends_with("ops/s") {
        (target.trim_end_matches("ops/s"), 1.0)
    } else if target.ends_with("MB/s") {
        (target.trim_end_matches("MB/s"), 1_000_000.0)
    } else {
        (target, 1.0)
    };
    
    num_str.parse::<f64>()
        .map(|v| v * multiplier)
        .map_err(|e| format!("Failed to parse target '{}': {}", target, e))
}

impl fmt::Display for BenchmarkResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = &self.unit;
        let (mean_str, unit_str) = format_value(self.mean, unit);
        let (median_str, _) = format_value(self.median, unit);
        let (p90_str, _) = format_value(self.p90, unit);
        let (p99_str, _) = format_value(self.p99, unit);
        let (p999_str, _) = format_value(self.p999, unit);

        writeln!(f, "Benchmark: {}", self.name)?;
        writeln!(f, "  Target:      {}", self.target.as_deref().unwrap_or("N/A"))?;
        writeln!(f, "  Samples:     {}", self.samples)?;
        writeln!(f, "  Mean:        {} {}", mean_str, unit_str)?;
        writeln!(f, "  Median:      {} {}", median_str, unit_str)?;
        writeln!(f, "  P90:         {} {}", p90_str, unit_str)?;
        writeln!(f, "  P99:         {} {}", p99_str, unit_str)?;
        writeln!(f, "  P99.9:       {} {}", p999_str, unit_str)?;
        writeln!(f, "  StdDev:      {:.2} {}", self.std_dev, unit_str)?;
        writeln!(f, "  Min:         {} {}", format_value(self.min, unit).0, unit_str)?;
        writeln!(f, "  Max:         {} {}", format_value(self.max, unit).0, unit_str)?;
        
        if let Some(tp) = self.throughput {
            writeln!(f, "  Throughput:  {:.2} ops/s", tp)?;
        }
        
        if let Some(passes) = self.passes_target() {
            writeln!(f, "  Target:      {}", if passes { "✓ PASS" } else { "✗ FAIL" })?;
        }
        
        Ok(())
    }
}

fn format_value(val: f64, unit: &str) -> (String, String) {
    match unit {
        "ns" => {
            if val < 1000.0 {
                (format!("{:.2}", val), "ns".to_string())
            } else if val < 1_000_000.0 {
                (format!("{:.2}", val / 1_000.0), "µs".to_string())
            } else if val < 1_000_000_000.0 {
                (format!("{:.2}", val / 1_000_000.0), "ms".to_string())
            } else {
                (format!("{:.2}", val / 1_000_000_000.0), "s".to_string())
            }
        }
        "µs" | "us" => {
            if val < 1000.0 {
                (format!("{:.2}", val), "µs".to_string())
            } else if val < 1_000_000.0 {
                (format!("{:.2}", val / 1_000.0), "ms".to_string())
            } else {
                (format!("{:.2}", val / 1_000_000.0), "s".to_string())
            }
        }
        "ms" => {
            if val < 1000.0 {
                (format!("{:.2}", val), "ms".to_string())
            } else {
                (format!("{:.2}", val / 1000.0), "s".to_string())
            }
        }
        "s" => (format!("{:.2}", val), "s".to_string()),
        _ => (format!("{:.2}", val), unit.to_string()),
    }
}

pub struct BenchmarkContext {
    pub runtime: Arc<Runtime>,
    pub temp_dir: PathBuf,
}

impl BenchmarkContext {
    pub fn new() -> anyhow::Result<Self> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?,
        );
        let temp_dir = tempfile::tempdir()?.into_path();
        Ok(Self { runtime, temp_dir })
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }
}

impl Default for BenchmarkContext {
    fn default() -> Self {
        Self::new().expect("Failed to create benchmark context")
    }
}

pub fn setup_criterion() -> Criterion {
    Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .sample_size(100)
        .warm_up_time(Duration::from_secs(3))
        .with_plots()
}

pub fn save_baseline(results: &[BenchmarkResult], path: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_baseline(path: &str) -> anyhow::Result<Vec<BenchmarkResult>> {
    let content = std::fs::read_to_string(path)?;
    let results: Vec<BenchmarkResult> = serde_json::from_str(&content)?;
    Ok(results)
}

pub fn compare_with_baseline(current: &[BenchmarkResult], baseline: &[BenchmarkResult], threshold: f64) -> Vec<ComparisonResult> {
    let baseline_map: HashMap<_, _> = baseline.iter().map(|r| (r.name.clone(), r)).collect();
    
    current.iter().filter_map(|curr| {
        baseline_map.get(&curr.name).map(|base| {
            let mut comparison = ComparisonResult {
                name: curr.name.clone(),
                unit: curr.unit.clone(),
                current_mean: curr.mean,
                baseline_mean: base.mean,
                current_p99: curr.p99,
                baseline_p99: base.p99,
                current_p999: curr.p999,
                baseline_p999: base.p999,
                mean_change_pct: 0.0,
                p99_change_pct: 0.0,
                p999_change_pct: 0.0,
                regressed: false,
            };
            
            if base.mean > 0.0 {
                comparison.mean_change_pct = ((curr.mean - base.mean) / base.mean) * 100.0;
            }
            if base.p99 > 0.0 {
                comparison.p99_change_pct = ((curr.p99 - base.p99) / base.p99) * 100.0;
            }
            if base.p999 > 0.0 {
                comparison.p999_change_pct = ((curr.p999 - base.p999) / base.p999) * 100.0;
            }
            
            comparison.regressed = comparison.p99_change_pct > threshold || comparison.p999_change_pct > threshold;
            
            comparison
        })
    }).collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub name: String,
    pub unit: String,
    pub current_mean: f64,
    pub baseline_mean: f64,
    pub current_p99: f64,
    pub baseline_p99: f64,
    pub current_p999: f64,
    pub baseline_p999: f64,
    pub mean_change_pct: f64,
    pub p99_change_pct: f64,
    pub p999_change_pct: f64,
    pub regressed: bool,
}

impl fmt::Display for ComparisonResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.regressed { "✗ REGRESSED" } else { "✓ OK" };
        writeln!(f, "{} {}", self.name, status)?;
        writeln!(f, "  Mean:   {:.2} -> {:.2} ({:+.1}%)", self.baseline_mean, self.current_mean, self.mean_change_pct)?;
        writeln!(f, "  P99:    {:.2} -> {:.2} ({:+.1}%)", self.baseline_p99, self.current_p99, self.p99_change_pct)?;
        writeln!(f, "  P99.9:  {:.2} -> {:.2} ({:+.1}%)", self.baseline_p999, self.current_p999, self.p999_change_pct)?;
        Ok(())
    }
}

pub fn print_comparison_report(comparisons: &[ComparisonResult], threshold: f64) {
    println!("\n=== Benchmark Comparison Report (threshold: {:.1}%) ===\n", threshold);
    
    let regressed: Vec<_> = comparisons.iter().filter(|c| c.regressed).collect();
    let passed: Vec<_> = comparisons.iter().filter(|c| !c.regressed).collect();
    
    if !regressed.is_empty() {
        println!("REGRESSIONS ({}):", regressed.len());
        for c in regressed {
            println!("{}", c);
        }
        println!();
    }
    
    println!("PASSED ({}):", passed.len());
    for c in passed {
        println!("{}", c);
    }
    
    println!("\n=== Summary ===");
    println!("Total: {}", comparisons.len());
    println!("Regressed: {}", regressed.len());
    println!("Passed: {}", passed.len());
}

pub struct HistogramRecorder {
    histogram: Histogram<u64>,
}

impl HistogramRecorder {
    pub fn new(max_value: u64, precision: u8) -> anyhow::Result<Self> {
        Ok(Self {
            histogram: Histogram::new_with_max(max_value, precision)?,
        })
    }

    pub fn record(&mut self, value: u64) {
        let _ = self.histogram.record(value);
    }

    pub fn record_duration(&mut self, duration: Duration) {
        self.record(duration.as_nanos() as u64);
    }

    pub fn get(&self) -> &Histogram<u64> {
        &self.histogram
    }

    pub fn into_result(self, name: &str, unit: &str, target: Option<&str>) -> BenchmarkResult {
        BenchmarkResult::from_histogram(name, &self.histogram, unit, target)
    }
}

#[macro_export]
macro_rules! bench_async {
    ($group:expr, $name:expr, $fut:expr) => {{
        let rt = tokio::runtime::Runtime::new().unwrap();
        $group.bench_function($name, |b| {
            b.to_async(&rt).iter(|| $fut);
        });
    }};
}

#[macro_export]
macro_rules! bench_async_with_input {
    ($group:expr, $name:expr, $input:expr, $fut:expr) => {{
        let rt = tokio::runtime::Runtime::new().unwrap();
        $group.bench_with_input($name, $input, |b, input| {
            b.to_async(&rt).iter(|| $fut(input));
        });
    }};
}