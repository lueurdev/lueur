pub mod bandwidth;
pub mod dns;
pub mod jitter;
pub mod latency;
pub mod packet_loss;

use std::sync::Arc;

use bandwidth::create_bandwidth_plugin;
use dns::create_dns_plugin;
use jitter::create_jitter_plugin;
use latency::create_latency_plugin;
use packet_loss::create_packet_loss_plugin;
use tokio::sync::RwLock;

use crate::config::FaultConfig;
use crate::config::ProxyConfig;
use crate::errors::ProxyError;
use crate::plugin::ProxyPlugin;

pub async fn load_builtin_plugins(
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Vec<Arc<dyn ProxyPlugin>>, ProxyError> {
    let mut plugins: Vec<Arc<dyn ProxyPlugin>> = Vec::new();

    let lock = config.read().await;
    let fault = lock.fault.clone();

    match fault {
        FaultConfig::Dns(settings) => plugins.push(create_dns_plugin(settings)),
        FaultConfig::Latency(settings) => {
            plugins.push(create_latency_plugin(settings))
        }
        FaultConfig::PacketLoss(settings) => {
            plugins.push(create_packet_loss_plugin(settings))
        }
        FaultConfig::Bandwidth(settings) => {
            plugins.push(create_bandwidth_plugin(settings))
        }
        FaultConfig::Jitter(settings) => {
            plugins.push(create_jitter_plugin(settings))
        }
    }

    Ok(plugins)
}
