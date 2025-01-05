use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::fmt::Write;

use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use tracing::info;
use colored::*;

use crate::errors::ScenarioError;
use crate::event::FaultEvent;
use crate::plugin::RemotePluginInfo;
use crate::types::Direction;
use crate::types::FaultConfiguration;

// see https://github.com/serde-rs/serde/issues/1560#issuecomment-1666846833
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReportItemExpectationDecision {
    Success,
    Failure,
    #[serde(untagged)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemHttpExpectation {
    pub status_code: Option<u16>,
    pub response_time_under: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemHttpResult {
    pub status_code: Option<u16>,
    pub response_time: Option<f64>,
    pub decision: ReportItemExpectationDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ReportItemExpectation {
    Http {
        wanted: ReportItemHttpExpectation,
        got: Option<ReportItemHttpResult>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemFault {
    pub event: FaultEvent,
    pub direction: Direction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemEvent {
    pub event: FaultEvent,
    pub direction: Direction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ReportItemProtocol {
    Http { code: u16, body_length: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportItemMetricsFaults {
    pub url: String,
    pub computed: Option<ReportItemFault>,
    pub applied: Option<Vec<ReportItemEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemMetrics {
    pub dns: Vec<DnsTiming>,
    pub protocol: Option<ReportItemProtocol>,
    pub ttfb: f64,
    pub total_time: f64,
    pub faults: Vec<ReportItemMetricsFaults>,
}

impl ReportItemMetrics {
    pub fn new() -> Self {
        ReportItemMetrics {
            dns: Vec::new(),
            ttfb: 0.0,
            total_time: 0.0,
            protocol: None,
            faults: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsTiming {
    pub host: String,
    pub duration: f64,
    pub resolved: bool,
}

impl DnsTiming {
    pub fn new() -> Self {
        DnsTiming { host: "".to_string(), duration: 0.0, resolved: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemTarget {
    pub address: String,
}

/// Report for a single entry in the scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemResult {
    pub target: ReportItemTarget,
    pub expect: Option<ReportItemExpectation>,
    pub fault: FaultConfiguration,
    pub metrics: Option<ReportItemMetrics>, // Metrics collected
    pub errors: Vec<String>,                // Errors encountered
    pub total_time: f64,                    // Total time in milliseconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub plugins: Vec<RemotePluginInfo>,
    pub items: Vec<ReportItemResult>, // Scenario entries report
}

impl Report {
    /// Saves the ScenarioReport to the specified file path in JSON or YAML
    /// format based on file extension.
    ///
    /// # Arguments
    ///
    /// * `path` - The file path where the report should be saved.
    ///
    /// # Returns
    ///
    /// * `Result<(), ScenarioError>` - Returns `Ok(())` if successful, or a
    ///   `ScenarioError` otherwise.
    pub fn save(&self, path: &str) -> Result<(), ScenarioError> {
        let file = File::create(path).map_err(|e| {
            error!("Failed to create report file '{}': {}", path, e);
            ScenarioError::IoError(e)
        })?;
        let writer = BufWriter::new(file);

        if path.ends_with(".json") {
            serde_json::to_writer_pretty(writer, self).map_err(|e| {
                error!("Failed to serialize report to JSON: {}", e);
                ScenarioError::ReportError(e.to_string())
            })?;
        } else if path.ends_with(".yaml") || path.ends_with(".yml") {
            serde_yaml::to_writer(writer, self).map_err(|e| {
                error!("Failed to serialize report to YAML: {}", e);
                ScenarioError::ReportError(e.to_string())
            })?;
        } else {
            let err_msg = "Unsupported report file format. Use .json or .yaml"
                .to_string();
            error!("{}", err_msg);
            return Err(ScenarioError::ReportError(err_msg));
        }

        info!("Report successfully saved to '{}'.", path);
        Ok(())
    }
}

/// Trait to capitalize the first letter of a string.
trait Capitalize {
    fn capitalize(&self) -> String;
}

impl Capitalize for String {
    fn capitalize(&self) -> String {
        let mut c = self.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }
}

/// Defines the latency-based SLO metrics.
const SLO_METRICS: &[(&str, f64)] = &[
    ("99% < 200ms", 200.0),
    ("95% < 500ms", 500.0),
    ("90% < 1s", 1000.0),
];

/// Defines the error rate-based SLO metrics.
const ERROR_RATE_SLOS: &[(&str, f64)] = &[
    ("99% < 1% Error Rate", 1.0),
    ("95% < 0.5% Error Rate", 0.5),
];

pub fn pretty_report(report: Report) -> Result<String, Box<dyn std::error::Error>> {

    let mut text: String = String::new();

    // Initialize counters for error rate calculations.
    let mut endpoint_request_counts: HashMap<String, usize> = HashMap::new();
    let mut endpoint_error_counts: HashMap<String, usize> = HashMap::new();

    // First pass to count total and error requests per endpoint.
    for item in &report.items {
        let endpoint = &item.target.address;
        *endpoint_request_counts.entry(endpoint.clone()).or_insert(0) += 1;
        match item.expect.as_ref().unwrap() {
            ReportItemExpectation::Http { wanted, got } => {
                if &got.as_ref().unwrap().decision == &ReportItemExpectationDecision::Failure {
                    *endpoint_error_counts.entry(endpoint.clone()).or_insert(0) += 1;
                }
            },
        }
    }

    // Print the Markdown table header.
    writeln!(text, "# Lueur Resilience Test Report\n");
    writeln!(text, 
        "| **Endpoint** | **Total Fault Injected** | **SLO: 99% < 200ms** | **SLO: 95% < 500ms** | **SLO: 90% < 1s** | **SLO: 99% < 1% Error Rate** | **SLO: 95% < 0.5% Error Rate** |"
    );
    writeln!(text, "|-------------|--------------------------|-----------------------|-----------------------|-----------------------|----------------------------------|-----------------------------------|");

    // Iterate over each test item to generate table rows.
    for item in &report.items {
        let endpoint = &item.target.address;
        let total_faults = summarize_faults(&item.fault, &item.metrics.as_ref().unwrap().faults);

        // Evaluate latency-based SLOs.
        let mut slo_results = Vec::new();
        for &(slo, threshold) in SLO_METRICS {
            let (status, breach_info) = evaluate_latency_slo(item, threshold);
            if let Some(info) = breach_info {
                slo_results.push(format!("{} {}", status, info));
            } else {
                slo_results.push(status);
            }
        }

        // Evaluate error rate-based SLOs.
        let total_requests = *endpoint_request_counts.get(endpoint).unwrap_or(&0);
        let error_requests = *endpoint_error_counts.get(endpoint).unwrap_or(&0);
        for &(slo, threshold) in ERROR_RATE_SLOS {
            let (status, breach_info) =
                evaluate_error_rate_slo(item, threshold, total_requests, error_requests);
            if let Some(info) = breach_info {
                slo_results.push(format!("{} {}", status, info));
            } else {
                slo_results.push(status);
            }
        }

        // Print the table row for the current endpoint.
        writeln!(
            text, 
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            endpoint,
            total_faults,
            slo_results[0],
            slo_results[1],
            slo_results[2],
            slo_results[3],
            slo_results[4],
        );
    }

    // Generate and print the aggregated summary.
    writeln!(text, "{}", generate_summary(&report));

    // Analyze and display fault types.
    writeln!(text, "{}", display_fault_analysis(&report));

    // Generate and print recommendations based on fault analysis.
    writeln!(text, "\n## Recommendations");
    let recommendations = generate_fault_recommendations(&report);
    writeln!(text, "{}", recommendations);

    Ok(text)
}

/// Summarizes all faults injected into an endpoint into a single string.
fn summarize_faults(fault: &FaultConfiguration, faults_applied: &Vec<ReportItemMetricsFaults>) -> String {
    let mut summary = String::new();

    summary.push_str(&format!("{}", fault));

    /* 
    // Additional faults.
    for applied_fault in faults_applied {
        let event_type = &applied_fault.computed.event.event_type;
        let delay = applied_fault.computed.event.delay;
        summary.push_str(&format!(
            "; {}: {:.0}{}",
            event_type,
            delay,
            match event_type.to_lowercase().as_str() {
                "bandwidth" => "kbps",
                "packet loss" => "%",
                "jitter" => "ms",
                "latency" => "ms",
                "http errors" => "",
                _ => "",
            }
        ));
    }*/

    summary
}

/// Evaluates latency-based SLOs and returns the status and breach details.
fn evaluate_latency_slo(item: &ReportItemResult, threshold: f64) -> (String, Option<String>) {
    match &item.expect {
        Some(expectation) => {
            match expectation {
                ReportItemExpectation::Http { wanted, got } => {
                    match got {
                        Some(result) => {
                            let actual_response_time = result.response_time.unwrap_or(0.0);
                            let decision = &result.decision;
                        
                            if decision == &ReportItemExpectationDecision::Success {
                                (format!("{}", "✅ Met".green()), None)
                            } else {
                                let delta = actual_response_time - threshold;
                                // Estimate breach time based on delta and threshold (simplistic assumption)
                                let breach_time = estimate_breach_time(delta, threshold);
                                (
                                    format!("{}", "❌ Breached".red()),
                                    Some(format!("(+{:.0}ms, {}/week)", delta, breach_time)),
                                )
                            }
                        }
                        None => ("".to_string(), Some("".to_string()))
                    }
                }
            }
        }
        None => ("".to_string(), Some("".to_string()))
    }
    
}

/// Evaluates error rate-based SLOs and returns the status and breach details.
fn evaluate_error_rate_slo(
    item: &ReportItemResult,
    threshold: f64,
    total_requests: usize,
    error_requests: usize,
) -> (String, Option<String>) {
    let error_rate = if total_requests > 0 {
        (error_requests as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    if error_rate <= threshold {
        (format!("{}", "✅ Met".green()), None)
    } else {
        let delta = error_rate - threshold;
        let breach_time = estimate_breach_time(delta, threshold);
        (
            format!("{}", "❌ Breached".red()),
            Some(format!("(+{:.1}% Error Rate, {}/week)", delta, breach_time)),
        )
    }
}

/// Estimates the breach duration based on the delta and threshold.
/// This is a simplistic estimation and can be refined with real traffic data.
fn estimate_breach_time(delta: f64, threshold: f64) -> String {
    // Calculate breach ratio.
    let breach_ratio = delta / threshold;

    // Assume an average request rate (e.g., 100 requests per second).
    let request_rate_per_sec = 100.0;

    // Calculate total requests per week.
    let total_requests = request_rate_per_sec * 7.0 * 24.0 * 60.0 * 60.0;

    // Estimate number of breached requests.
    let breached_requests = breach_ratio * total_requests;

    // Estimate total breach time in seconds.
    let breach_seconds = breached_requests * (delta / 1000.0); // converting ms to seconds

    let hours = (breach_seconds / 3600.0).floor();
    let minutes = ((breach_seconds % 3600.0) / 60.0).floor();
    let seconds = (breach_seconds % 60.0).floor();
    format!("{}h {}m {}s", hours, minutes, seconds)
}

/// Generates and prints an aggregated summary of the test results.
fn generate_summary(report: &Report) -> String {
    let total_tests = report.items.len();
    let total_failures = report
        .items
        .iter()
        .filter(|item| 
            match item.expect.as_ref().unwrap() {
                ReportItemExpectation::Http { wanted, got } => {
                    let decision = &got.as_ref().unwrap().decision;
                    decision == &ReportItemExpectationDecision::Failure
                }
            }
        ).count();

    let mut summary = String::new();

    writeln!(summary, "\n## Summary");
    writeln!(
        summary, 
        "- **Total Test Cases:** {}",
        total_tests.to_string().cyan().bold()
    );
    writeln!(
        summary, 
        "- **Failures:** {}",
        total_failures.to_string().red().bold()
    );

    if total_failures > 0 {
        writeln!(
            summary, 
            "- {}: Investigate the failed test cases to enhance your application's resilience.",
            "🔍 Recommendation".yellow().bold()
        );
    } else {
        writeln!(
            summary, 
            "- {}: Excellent job! All test cases passed successfully.",
            "🌟 Recommendation".green().bold()
        );
    }

    summary
}

/// Analyzes and counts the occurrences of each fault type.
fn analyze_fault_types(report: &Report) -> HashMap<String, usize> {
    let mut fault_counts = HashMap::new();

    for item in &report.items {
        let primary_fault = item.fault.fault_type();
        *fault_counts.entry(primary_fault).or_insert(0) += 1;

        for fault_detail in &item.metrics.as_ref().unwrap().faults {
            let additional_fault = fault_detail.computed.as_ref().unwrap().event.event_type();
            *fault_counts.entry(additional_fault).or_insert(0) += 1;
        }
    }

    fault_counts
}

/// Displays the fault type analysis in the report.
fn display_fault_analysis(report: &Report) -> String {
    let mut text: String = String::new();

    let fault_counts = analyze_fault_types(report);
    writeln!(text, "\n## Fault Type Analysis");
    for (fault_type, count) in fault_counts {
        writeln!(
            text,
            "- **{}** occurred {} times.",
            fault_type.capitalize(),
            count
        );
    }

    text
}

/// Generates and returns fault-based recommendations based on fault type frequencies.
fn generate_fault_recommendations(report: &Report) -> String {
    let fault_counts = analyze_fault_types(report);
    let mut recommendations = String::new();

    for (fault_type, count) in fault_counts {
        match fault_type.as_str() {
            "latency" if count > 3 => {
                recommendations.push_str(&format!(
                    "{}: High latency issues detected frequently. Consider optimizing network calls or improving service scalability.\n",
                    "🔧 Recommendation".yellow().bold()
                ));
            }
            "packet loss" if count > 1 => {
                recommendations.push_str(&format!(
                    "{}: Packet loss observed in multiple endpoints. Investigate network stability and routing paths.\n",
                    "🔧 Recommendation".yellow().bold()
                ));
            }
            "http errors" if count > 1 => {
                recommendations.push_str(&format!(
                    "{}: Frequent HTTP errors detected. Review error handling and server configurations.\n",
                    "🔧 Recommendation".yellow().bold()
                ));
            }
            "bandwidth" if count > 2 => {
                recommendations.push_str(&format!(
                    "{}: Bandwidth limitations impacting multiple endpoints. Consider upgrading network infrastructure or optimizing data transfer.\n",
                    "🔧 Recommendation".yellow().bold()
                ));
            }
            "jitter" if count > 2 => {
                recommendations.push_str(&format!(
                    "{}: Jitter affecting endpoint performance. Implement measures to stabilize network latency.\n",
                    "🔧 Recommendation".yellow().bold()
                ));
            }
            _ => {}
        }
    }

    if recommendations.is_empty() {
        recommendations.push_str("No specific recommendations based on fault types.\n");
    }

    recommendations
}
