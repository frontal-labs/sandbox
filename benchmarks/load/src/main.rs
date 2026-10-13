use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use sandbox_types::{SessionConfig, TierPreference};
use std::time::{Duration, Instant};
use tokio::time::sleep;

mod scenarios;
mod reporter;

use scenarios::{ScenarioConfig, ScenarioType};
use reporter::{ReportGenerator, ScenarioResult};

#[derive(Parser)]
#[command(name = "sandbox-load")]
#[command(about = "Load testing tool for Frontal Sandbox", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a load test scenario
    Run {
        /// Scenario to run
        #[arg(short, long, default_value = "mixed")]
        scenario: String,
        
        /// Number of concurrent workers
        #[arg(short, long, default_value = "100")]
        concurrency: usize,
        
        /// Duration in seconds
        #[arg(short, long, default_value = "30")]
        duration: u64,
        
        /// Ramp-up time in seconds
        #[arg(long, default_value = "5")]
        ramp_up: u64,
        
        /// Output format (json, csv, text)
        #[arg(short, long, default_value = "text")]
        format: String,
        
        /// Output file
        #[arg(short, long)]
        output: Option<String>,
        
        /// Base URL for server mode (if not using embedded)
        #[arg(long)]
        server_url: Option<String>,
    },
    
    /// Compare current results with baseline
    Compare {
        /// Current results file
        current: String,
        
        /// Baseline results file
        baseline: String,
        
        /// Regression threshold percentage
        #[arg(long, default_value = "10.0")]
        threshold: f64,
    },
    
    /// List available scenarios
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Run { scenario, concurrency, duration, ramp_up, format, output, server_url } => {
            run_scenario(scenario, concurrency, duration, ramp_up, format, output, server_url).await?;
        }
        Commands::Compare { current, baseline, threshold } => {
            compare_results(current, baseline, threshold).await?;
        }
        Commands::List => {
            list_scenarios();
        }
    }
    
    Ok(())
}

async fn run_scenario(
    scenario_name: String,
    concurrency: usize,
    duration: u64,
    ramp_up: u64,
    format: String,
    output: Option<String>,
    server_url: Option<String>,
) -> anyhow::Result<()> {
    println!("Starting load test: {}", scenario_name);
    println!("  Concurrency: {}", concurrency);
    println!("  Duration: {}s", duration);
    println!("  Ramp-up: {}s", ramp_up);
    
    let config = ScenarioConfig {
        concurrency,
        duration: Duration::from_secs(duration),
        ramp_up: Duration::from_secs(ramp_up),
        server_url,
    };
    
    // Clone config for the scenario
    let scenario = scenarios::get_scenario(&scenario_name, config.clone())?;
    
    let progress = ProgressBar::new(duration);
    progress.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s ({eta})")
        .unwrap());
    
    let start_time = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    
    // Run the scenario in a separate task
    let scenario_result = tokio::spawn(async move {
        scenario.run(config).await
    });
    
    let result = tokio::select! {
        result = scenario_result => {
            progress.finish_and_clear();
            result?? // Result from join, then Result from scenario
        }
        _ = async {
            loop {
                interval.tick().await;
                progress.inc(1);
            }
        } => unreachable!(),
    };
    
    let elapsed = start_time.elapsed();
    
    println!("\n=== Load Test Results ===");
    println!("Scenario:     {}", scenario_name);
    println!("Duration:     {:.2}s", elapsed.as_secs_f64());
    println!("Concurrency:  {}", concurrency);
    println!();
    
    result.print_summary();
    
    if let Some(output_path) = output {
        let generator = ReportGenerator::new(&format);
        generator.write(&result, &output_path)?;
        println!("\nResults written to: {}", output_path);
    }
    
    Ok(())
}

async fn compare_results(current: String, baseline: String, threshold: f64) -> anyhow::Result<()> {
    let current_data = std::fs::read_to_string(&current)?;
    let baseline_data = std::fs::read_to_string(&baseline)?;
    
    let current_results: ScenarioResult = serde_json::from_str(&current_data)?;
    let baseline_results: ScenarioResult = serde_json::from_str(&baseline_data)?;
    
    println!("=== Comparison Report ===");
    println!("Current:  {}", current);
    println!("Baseline: {}", baseline);
    println!("Threshold: {:.1}%\n", threshold);
    
    let comparisons = compare_metrics(&current_results, &baseline_results, threshold);
    
    let mut regressed = 0;
    for comp in &comparisons {
        let status = if comp.regressed { "✗ REGRESSED" } else { "✓ OK" };
        println!("{} {:.2} -> {:.2} ({:+.1}%) [{}]", 
            comp.name, comp.baseline, comp.current, comp.change_pct, status);
        if comp.regressed {
            regressed += 1;
        }
    }
    
    println!("\n=== Summary ===");
    println!("Total metrics: {}", comparisons.len());
    println!("Regressed:     {}", regressed);
    println!("Passed:        {}", comparisons.len() - regressed);
    
    if regressed > 0 {
        std::process::exit(1);
    }
    
    Ok(())
}

fn compare_metrics(current: &ScenarioResult, baseline: &ScenarioResult, threshold: f64) -> Vec<MetricComparison> {
    let mut comparisons = Vec::new();
    
    // Compare latency percentiles
    for (name, curr_lat) in &current.latencies {
        if let Some(base_lat) = baseline.latencies.get(name) {
            comparisons.push(MetricComparison {
                name: format!("latency.{}", name),
                current: curr_lat.percentiles.get(&99).copied().unwrap_or(0) as f64,
                baseline: base_lat.percentiles.get(&99).copied().unwrap_or(0) as f64,
                change_pct: if base_lat.percentiles.get(&99).copied().unwrap_or(0) > 0 {
                    let base = base_lat.percentiles.get(&99).copied().unwrap_or(0) as f64;
                    let curr = curr_lat.percentiles.get(&99).copied().unwrap_or(0) as f64;
                    ((curr - base) / base) * 100.0
                } else { 0.0 },
                regressed: if base_lat.percentiles.get(&99).copied().unwrap_or(0) > 0 {
                    let base = base_lat.percentiles.get(&99).copied().unwrap_or(0) as f64;
                    let curr = curr_lat.percentiles.get(&99).copied().unwrap_or(0) as f64;
                    ((curr - base) / base) * 100.0 > threshold
                } else { false },
            });
        }
    }
    
    // Compare throughput
    for (name, curr_tp) in &current.throughput {
        if let Some(base_tp) = baseline.throughput.get(name) {
            comparisons.push(MetricComparison {
                name: format!("throughput.{}", name),
                current: *curr_tp,
                baseline: *base_tp,
                change_pct: if *base_tp > 0.0 { ((*curr_tp - *base_tp) / *base_tp) * 100.0 } else { 0.0 },
                regressed: *curr_tp < *base_tp * (1.0 - threshold / 100.0),
            });
        }
    }
    
    // Compare error rates
    for (name, curr_err) in &current.error_rates {
        if let Some(base_err) = baseline.error_rates.get(name) {
            comparisons.push(MetricComparison {
                name: format!("error_rate.{}", name),
                current: *curr_err,
                baseline: *base_err,
                change_pct: if *base_err > 0.0 { ((curr_err - base_err) / base_err) * 100.0 } else { 0.0 },
                regressed: *curr_err > *base_err * (1.0 + threshold / 100.0),
            });
        }
    }
    
    comparisons
}

#[derive(Debug)]
struct MetricComparison {
    name: String,
    current: f64,
    baseline: f64,
    change_pct: f64,
    regressed: bool,
}

fn list_scenarios() {
    println!("Available scenarios:");
    println!("  concurrent_sessions  - Test concurrent idle sessions (memory)");
    println!("  exec_throughput      - Maximum exec/sec throughput");
    println!("  snapshot_fork        - Snapshot and fork under load");
    println!("  mixed_workload       - Realistic agent workload (git, npm, test)");
}