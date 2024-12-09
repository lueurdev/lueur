// src/plugin/fault/latency.rs

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

use crate::config::LatencySettings;
use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::latency::LatencyInjector;
use crate::fault::latency::LatencyOptions;
use crate::fault::latency::LatencyStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::LatencyDistribution;

/// LatencyFaultPlugin is a plugin that injects latency into streams.
#[derive(Debug)]
pub struct LatencyFaultPlugin {
    injector: Arc<dyn FaultInjector>,
}

impl LatencyFaultPlugin {
    pub fn new_from_settings(settings: LatencySettings) -> Self {
        let latency_options = LatencyOptions {
            strategy: match settings.distribution {
                LatencyDistribution::Normal => LatencyStrategy::Normal {
                    mean: settings.latency_mean,
                    stddev: settings.latency_stddev,
                },
                LatencyDistribution::Uniform => LatencyStrategy::Uniform {
                    min: settings.latency_min,
                    max: settings.latency_max,
                },
                LatencyDistribution::Pareto => LatencyStrategy::Pareto {
                    shape: settings.latency_shape,
                    scale: settings.latency_scale,
                },
                LatencyDistribution::ParetoNormal => {
                    LatencyStrategy::ParetoNormal {
                        shape: settings.latency_shape,
                        scale: settings.latency_scale,
                        mean: settings.latency_mean,
                        stddev: settings.latency_stddev,
                    }
                }
            },
        };
        let injector = Arc::new(LatencyInjector::new(latency_options));
        Self { injector }
    }
}

#[async_trait]
impl ProxyPlugin for LatencyFaultPlugin {
    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn process_request(
        &self,
        req: ReqwestRequest,
    ) -> Result<ReqwestRequest, ProxyError> {
        Ok(req)
    }

    async fn process_response(
        &self,
        resp: ReqwestResponse,
    ) -> Result<ReqwestResponse, ProxyError> {
        self.injector.apply_on_response(resp).await
    }

    async fn process_connect_request(
        &self,
        req: ConnectRequest,
    ) -> Result<ConnectRequest, ProxyError> {
        Ok(req)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
    ) -> Result<(), ProxyError> {
        Ok(())
    }

    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    > {
        let injected_client = self.injector.inject(client_stream);
        let injected_server = self.injector.inject(server_stream);

        Ok((injected_client, injected_server))
    }
}

/// Factory function to create a jitter fault plugin.
pub fn create_latency_plugin(
    settings: LatencySettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(LatencyFaultPlugin::new_from_settings(settings))
}
