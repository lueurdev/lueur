use std::collections::HashMap;
use std::fs::read_to_string;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use reqwest::RequestBuilder;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::watch;

use crate::cli::Spinner;
use crate::cli::set_spinner_message;
use crate::config;
use crate::config::FaultConfig;
use crate::config::ProxyConfig;
use crate::errors::ProxyError;
use crate::errors::ScenarioError;
use crate::reporting::DnsTiming;
use crate::reporting::Report;
use crate::reporting::ReportItem;
use crate::reporting::ReportItemRequest;
use crate::reporting::ReportItemRequestMetrics;
use crate::reporting::ReportItemResult;
use crate::resolver::TimingResolver;
use crate::types::FaultConfiguration;
use crate::types::LatencyDistribution;
use crate::types::PacketLossType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRequestExpectation {
    pub status: Option<u16>,
    pub response_time_under: Option<f64>, // ms
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRequest {
    pub method: String, // HTTP method (e.g., GET, POST)
    pub url: String,    // Target URL
    pub headers: Option<HashMap<String, String>>, // Optional headers
    pub body: Option<String>, // Optional request body
    pub expect: Option<ScenarioRequestExpectation>,
}

/// A single entry in the scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioItem {
    pub title: String,
    pub description: String,
    pub fault: FaultConfiguration, // Fault configuration
    pub requests: Vec<ScenarioRequest>, // List of requests
    pub concurrent: bool,          // Run requests concurrently or sequentially
}

/// The overall scenario containing multiple entries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub proxy: ScenarioProxyConfig,
    pub plugins: Vec<ScenarioPluginConfig>,
    pub upstream_hosts: Vec<String>,
    pub items: Vec<ScenarioItem>, // List of scenario entries
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScenarioProxyConfig {
    pub address: Option<String>, // Allow optional to enable CLI override
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScenarioPluginConfig {
    pub address: String, // Required
}

impl Scenario {
    /// Executes the scenario and generates a report
    pub async fn execute(
        &self,
        spinner: Option<&Spinner>,
        config_tx: watch::Sender<ProxyConfig>,
    ) -> Result<Report, ScenarioError> {
        let started_dt = Utc::now();

        let proxy_address = self.proxy.address.clone().unwrap();
        let _plugin_addresses: Vec<String> =
            self.plugins.iter().map(|c| c.address.clone()).collect();

        set_spinner_message(spinner, "Gathering gRPC plugins information");

        /*let mut plugin_infos: Vec<RemotePluginInfo> = Vec::new();
        for plugin in &plugins_middlewares {
            match plugin.info().await {
                Ok(plugin_info) => plugin_infos.push(plugin_info.clone()),
                Err(_) => tracing::warn!("Plugin not online: {}", plugin.addr),
            }
        }*/

        set_spinner_message(spinner, "Starting proxy");

        let mut report_entries = Vec::new();

        for entry in &self.items {
            let title = &entry.title;
            let desc = &entry.description;

            set_spinner_message(spinner, format!("Running {}", title).as_str());

            let fault_config = build_fault_config(entry.fault.clone()).unwrap();
            let fault_type = fault_config.to_string();

            let new_config = ProxyConfig::new(fault_config.clone()).unwrap();

            if config_tx.send(new_config).is_err() {
                tracing::error!("Proxy task has been shut down.");
            }

            // Execute requests
            let mut results: Vec<ReportItem> = Vec::new();
            let mut errors = Vec::new();

            let mut report_item = ReportItem::new(
                fault_type.clone(),
                title.clone(),
                desc.clone(),
            );

            if entry.concurrent {
                let tasks: Vec<_> = entry
                    .requests
                    .iter()
                    .map(|request| {
                        let proxy_address = proxy_address.clone();
                        async move {
                            execute_request(
                                request.clone(),
                                proxy_address.clone(),
                            )
                            .await
                        }
                    })
                    .collect();

                for result in futures::future::join_all(tasks).await {
                    match result {
                        Ok(request_result) => {
                            report_item.metrics.push(request_result);
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
            } else {
                for request in &entry.requests {
                    match execute_request(
                        request.clone(),
                        proxy_address.clone(),
                    )
                    .await
                    {
                        Ok(request_result) => {
                            report_item.metrics.push(request_result);
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
            }

            results.push(report_item);

            // Collect metrics and errors into the report entry
            let total_time: f64 = results
                .iter()
                .map(|r| r.metrics.iter().map(|s| s.total).sum::<f64>())
                .sum::<f64>();
            report_entries.push(ReportItemResult {
                fault: entry.fault.clone(),
                metrics: results,
                errors,
                total_time,
            });
        }

        let finished_dt = Utc::now();

        Ok(Report {
            started: started_dt,
            completed: finished_dt,
            items: report_entries,
            plugins: Vec::new(),
        })
    }
}

/// Executes a single HTTP request through the proxy and collects metrics
pub async fn execute_request(
    request: ScenarioRequest,
    proxy_address: String,
) -> Result<ReportItemRequestMetrics, ProxyError> {
    // Track start times for individual metrics
    let start_time = Instant::now();

    let dns_timing = Arc::new(Mutex::new(DnsTiming::new()));

    let resolver = Arc::new(TimingResolver::new(dns_timing.clone()));
    let client = Arc::new(
        reqwest::Client::builder()
            .dns_resolver(resolver)
            .proxy(reqwest::Proxy::http(&proxy_address).unwrap())
            .build()
            .unwrap(),
    );

    // Build the reqwest request
    let reqwest_request =
        build_request(&client, &request)?.build().map_err(|e| {
            ProxyError::InvalidConfiguration(format!(
                "Failed to build request: {}",
                e
            ))
        })?;

    let mut expected_status: Option<u16> = None;
    let mut expected_max_response_time: Option<f64> = None;

    if request.expect.is_some() {
        let expectation = request.expect.unwrap();
        expected_status = expectation.status;
        expected_max_response_time = expectation.response_time_under;
    }

    let req_metric_info = ReportItemRequest::new(
        &reqwest_request,
        expected_status,
        expected_max_response_time,
    );

    let conn_start = Instant::now();
    let ttfb_start = Instant::now();
    let response = client
        .execute(reqwest_request)
        .await
        .map_err(ProxyError::NetworkError)?;
    let conn_duration = conn_start.elapsed();

    let status = response.status();
    let ttfb_time = ttfb_start.elapsed();

    let body_bytes = response.bytes().await.unwrap();
    let body_length = body_bytes.len();
    let ttfb_duration = start_time.elapsed();

    let mut report = ReportItemRequestMetrics::new();
    report.connection_time = conn_duration.as_secs_f64();
    report.status = status.as_u16();
    report.ttfb = ttfb_time.as_secs_f64();
    report.total = ttfb_duration.as_secs_f64();
    report.body_length = body_length;
    report.expectation_met = match req_metric_info.clone().response_expectation
    {
        Some(r) => {
            let mut met = None;

            met = match r.status {
                Some(v) => Some(v == report.status),
                None => met,
            };

            met = match r.response_time_under {
                Some(v) => Some(report.total <= v / 1000.0),
                None => met,
            };

            met
        }
        None => None,
    };
    report.request = Some(req_metric_info);

    {
        let timing_lock = dns_timing.lock().await;
        report.dns = timing_lock.clone();
    }

    Ok(report)
}

fn build_request(
    client: &Arc<Client>,
    request: &ScenarioRequest,
) -> Result<RequestBuilder, ProxyError> {
    let mut req_builder = client.request(
        reqwest::Method::from_bytes(request.method.as_bytes()).map_err(
            |_| {
                ProxyError::InvalidConfiguration(
                    "Invalid HTTP method.".to_string(),
                )
            },
        )?,
        &request.url,
    );

    if let Some(headers) = &request.headers {
        for (key, value) in headers.iter() {
            req_builder = req_builder.header(key, value);
        }
    }

    if let Some(body) = &request.body {
        req_builder = req_builder.body(body.clone());
    }

    Ok(req_builder)
}

pub fn load_scenario_file(path: &str) -> Result<Scenario, ScenarioError> {
    let content = read_to_string(path)?;

    // Determine file type based on extension
    if path.ends_with(".json") {
        serde_json::from_str(&content).map_err(|e| {
            ScenarioError::LoadError(format!(
                "Failed to parse scenario as JSON: {}",
                e
            ))
        })
    } else if path.ends_with(".yaml") || path.ends_with(".yml") {
        serde_yaml::from_str(&content).map_err(|e| {
            ScenarioError::LoadError(format!(
                "Failed to parse scenario as YAML: {}",
                e
            ))
        })
    } else {
        Err(ScenarioError::LoadError(
            "Unsupported scenario file format. Use JSON or YAML.".to_string(),
        ))
    }
}

pub fn build_fault_config(
    fault: FaultConfiguration,
) -> Result<FaultConfig, ScenarioError> {
    match fault {
        FaultConfiguration::Bandwidth { bandwidth_rate } => {
            let settings = config::BandwidthSettings { bandwidth_rate };

            Ok(FaultConfig::Bandwidth(settings))
        }
        FaultConfiguration::Latency {
            latency_distribution,
            latency_mean,
            latency_stddev,
            latency_min,
            latency_max,
            latency_scale,
            latency_shape,
        } => {
            let settings = config::LatencySettings {
                distribution: LatencyDistribution::from_str(
                    &latency_distribution,
                )
                .unwrap(),
                latency_mean,
                latency_stddev,
                latency_min,
                latency_max,
                latency_shape,
                latency_scale,
            };

            Ok(FaultConfig::Latency(settings))
        }
        FaultConfiguration::PacketLoss {
            packet_loss_type,
            packet_loss_rate,
        } => {
            let settings = config::PacketLossSettings {
                loss_type: PacketLossType::from_str(&packet_loss_type).unwrap(),
                packet_loss_rate,
            };

            Ok(FaultConfig::PacketLoss(settings))
        }
        FaultConfiguration::Jitter { jitter_amplitude, jitter_frequency } => {
            let settings =
                config::JitterSettings { jitter_amplitude, jitter_frequency };

            Ok(FaultConfig::Jitter(settings))
        }
        FaultConfiguration::Dns { dns_rate } => {
            let settings = config::DnsSettings { dns_rate };

            Ok(FaultConfig::Dns(settings))
        }
    }
}
