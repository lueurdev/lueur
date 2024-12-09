use std::fs::File;
use std::io::BufWriter;

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use tracing::info;

use crate::errors::ScenarioError;
use crate::plugin::RemotePluginInfo;
use crate::types::FaultConfiguration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItem {
    pub fault_type: String,
    pub title: String,
    pub description: String,
    pub metrics: Vec<ReportItemRequestMetrics>,
}

impl ReportItem {
    pub fn new(fault_type: String, title: String, description: String) -> Self {
        ReportItem { fault_type, title, description, metrics: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemResponseExpectation {
    pub status: Option<u16>,
    pub response_time_under: Option<f64>, // ms
}

impl ReportItemResponseExpectation {
    pub fn new(
        expected_status: Option<u16>,
        expected_max_response_time: Option<f64>,
    ) -> Self {
        ReportItemResponseExpectation {
            status: expected_status,
            response_time_under: expected_max_response_time,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemRequest {
    pub method: String,
    pub url: String,
    pub response_expectation: Option<ReportItemResponseExpectation>,
}

impl ReportItemRequest {
    pub fn new(
        req: &reqwest::Request,
        expected_status: Option<u16>,
        expected_max_response_time: Option<f64>,
    ) -> Self {
        ReportItemRequest {
            method: req.method().to_string(),
            url: req.url().to_string(),
            response_expectation: Some(ReportItemResponseExpectation::new(
                expected_status,
                expected_max_response_time,
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemRequestMetrics {
    pub dns: DnsTiming,
    pub connection_time: f64,
    pub status: u16,
    pub ttfb: f64,
    pub total: f64,
    pub body_length: usize,
    pub request: Option<ReportItemRequest>,
    pub expectation_met: Option<bool>,
}

impl ReportItemRequestMetrics {
    pub fn new() -> Self {
        ReportItemRequestMetrics {
            dns: DnsTiming::new(),
            connection_time: 0.0,
            status: 200,
            ttfb: 0.,
            total: 0.0,
            body_length: 0,
            request: None,
            expectation_met: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsTiming {
    pub host: String,
    pub duration: f64,
}

impl DnsTiming {
    pub fn new() -> Self {
        DnsTiming { host: "".to_string(), duration: 0.0 }
    }
}

/// Report for a single entry in the scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItemResult {
    pub fault: FaultConfiguration, // Fault configuration
    pub metrics: Vec<ReportItem>,  // Metrics collected
    pub errors: Vec<String>,       // Errors encountered
    pub total_time: f64,           // Total time in seconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub started: DateTime<Utc>,
    pub completed: DateTime<Utc>,
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
