// used by duration to access as_millis_f64() which is available in edition2024
#![feature(duration_millis_float)]

mod cli;
mod config;
mod ebpf;
mod errors;
mod fault;
mod logging;
mod nic;
mod plugin;
mod proxy;
mod reporting;
mod resolver;
mod scenario;
mod types;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use cli::Commands;
use cli::RunCommands;
use cli::ScenarioCommands;
use cli::Spinner;
use config::ProxyConfig;
use errors::ProxyError;
use logging::init_logging;
use proxy::ProxyState;
use proxy::run_proxy;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tracing::debug;
use tracing::error;

use crate::scenario::load_scenario_file;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let _log_guard = init_logging(&cli.log_file).unwrap();

    // Initialize shared state with empty configuration
    let state = Arc::new(ProxyState::new(cli.ebpf));

    // Create a watch channel for configuration updates
    let (config_tx, config_rx) = watch::channel(ProxyConfig::default());

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1); // Capacity of 1

    // Create a oneshot channel for readiness signaling
    let (readiness_tx, readiness_rx) = oneshot::channel::<()>();

    let proxy_address = cli.proxy_address.clone();
    let upstream_hosts = cli.upstream_hosts.clone();

    state.update_upstream_hosts(upstream_hosts.clone()).await;

    let proxy_state = state.clone();

    let proxy_nic_config =
        nic::determine_proxy_and_ebpf_config(proxy_address, cli.iface).unwrap();

    let proxy_address = proxy_nic_config.proxy_address();

    tokio::spawn(run_proxy(
        proxy_address.clone(),
        proxy_state,
        shutdown_rx,
        readiness_tx,
        config_rx,
    ));

    // Wait for the proxy to signal readiness
    readiness_rx.await.map_err(|e| {
        ProxyError::Internal(format!(
            "Failed to receive readiness signal: {}",
            e
        ))
    })?;

    #[allow(unused_variables)]
    let ebpf_guard = match cli.ebpf {
        true => {
            let mut bpf = aya::Ebpf::load(aya::include_bytes_aligned!(
                concat!(env!("OUT_DIR"), "/lueur-ebpf")
            ))
            .unwrap();

            if let Err(e) = aya_log::EbpfLogger::init(&mut bpf) {
                tracing::warn!("failed to initialize eBPF logger: {}", e);
            }

            let _ = ebpf::install_and_run(
                &mut bpf,
                &proxy_nic_config,
                upstream_hosts.clone(),
            );

            tracing::info!("Ebpf has been loaded");

            Some(bpf)
        }
        false => None,
    };

    tracing::info!("Proxy server is running at {}", proxy_address);

    match cli.command {
        Commands::Run(run_cmd) => {
            let cmd_config = get_run_command_proxy_config(run_cmd).unwrap();
            if config_tx.send(cmd_config).is_err() {
                error!("Proxy task has been shut down.");
            }
        }
        Commands::Scenario(scenario_cmd) => match scenario_cmd {
            ScenarioCommands::Run(config) => {
                let spinner = Spinner::new("Starting the scenario...");

                match load_scenario_file(config.scenario.as_str()) {
                    Ok(scenario) => {
                        match scenario
                            .execute(Some(&spinner), config_tx.clone())
                            .await
                        {
                            Ok(report) => {
                                debug!("Scenario executed successfully!");

                                report.save(config.report.as_str())?;
                            }
                            Err(e) => {
                                error!("Failed to execute scenario: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to load scenario: {:?}", e);
                    }
                }
            }
        },
    }

    tokio::signal::ctrl_c().await.map_err(|e| {
        ProxyError::Internal(format!(
            "Failed to listen for shutdown signal: {}",
            e
        ))
    })?;

    tracing::info!("Shutdown signal received. Initiating shutdown.");

    // Send shutdown signal
    let _ = shutdown_tx.send(());

    Ok(())
}

/// Handles the 'Run' subcommands
fn get_run_command_proxy_config(
    run_cmd: RunCommands,
) -> Result<config::ProxyConfig, ProxyError> {
    match run_cmd {
        RunCommands::Dns(run_cmd_dns) => {
            let config = run_cmd_dns.config;
            Ok(config::ProxyConfig::new_dns(config).unwrap())
        }
        RunCommands::Latency(run_cmd_latency) => {
            let config = run_cmd_latency.config;
            Ok(config::ProxyConfig::new_latency(config).unwrap())
        }
        RunCommands::PacketLoss(run_cmd_packet_loss) => {
            let config = run_cmd_packet_loss.config;
            Ok(config::ProxyConfig::new_packet_loss(config).unwrap())
        }
        RunCommands::Bandwidth(run_cmd_bandwidth) => {
            let config = run_cmd_bandwidth.config;
            Ok(config::ProxyConfig::new_bandwidth(config).unwrap())
        }
        RunCommands::Jitter(run_cmd_jitter) => {
            let config = run_cmd_jitter.config;
            Ok(config::ProxyConfig::new_jitter(config).unwrap())
        }
    }
}
