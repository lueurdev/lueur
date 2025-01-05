// src/plugin/fault/latency.rs

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http;
use reqwest::Request as ReqwestRequest;

use crate::config::LatencySettings;
use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::latency::LatencyInjector;
use crate::fault::latency::LatencyOptions;
use crate::fault::latency::LatencyStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::Direction;
use crate::types::LatencyDistribution;

/// LatencyFaultPlugin is a plugin that injects latency into streams.
#[derive(Debug)]
pub struct LatencyFaultPlugin {
    injector: Arc<dyn FaultInjector>,
    direction: Direction,
}

impl fmt::Display for LatencyFaultPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Latency Plugin")
    }
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
        Self { injector, direction: settings.direction }
    }
}

#[async_trait]
impl ProxyPlugin for LatencyFaultPlugin {
    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn process_request(
        &self,
        req: ReqwestRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError> {
        Ok(req)
    }

    async fn process_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError> {
        self.injector.apply_on_response(resp, event).await
    }

    async fn process_connect_request(
        &self,
        req: ConnectRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ConnectRequest, ProxyError> {
        Ok(req)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<(), ProxyError> {
        Ok(())
    }

    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    > {
        let mut injected_client = client_stream;
        if self.direction.is_ingress() {
            injected_client = self.injector.inject(
                injected_client,
                &self.direction,
                event.clone(),
            );
        }

        let mut injected_server = server_stream;
        if self.direction.is_egress() {
            injected_server = self.injector.inject(
                injected_server,
                &self.direction,
                event.clone(),
            );
        }

        Ok((injected_client, injected_server))
    }
}

/// Factory function to create a jitter fault plugin.
pub fn create_latency_plugin(
    settings: LatencySettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(LatencyFaultPlugin::new_from_settings(settings))
}
