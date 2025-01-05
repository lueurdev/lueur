use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http;
use reqwest::Request as ReqwestRequest;

use crate::config::PacketLossSettings;
use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::packet_loss::PacketLossInjector;
use crate::fault::packet_loss::PacketLossOptions;
use crate::fault::packet_loss::PacketLossStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::Direction;
use crate::types::PacketLossType;

/// PacketLossFaultPlugin is a plugin that injects packet loss into streams
/// based on configured settings.
#[derive(Debug)]
pub struct PacketLossFaultPlugin {
    injector: Arc<dyn FaultInjector>,
    direction: Direction,
}

impl fmt::Display for PacketLossFaultPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Packet Loss Plugin")
    }
}

impl PacketLossFaultPlugin {
    pub fn new_from_settings(settings: PacketLossSettings) -> Self {
        let packet_loss_options = PacketLossOptions {
            strategy: match settings.loss_type {
                PacketLossType::Bernoulli => PacketLossStrategy::Bernoulli {
                    loss_probability: settings.packet_loss_rate,
                },
                PacketLossType::GilbertElliott => {
                    PacketLossStrategy::GilbertElliott {
                        loss_probability: settings.packet_loss_rate,
                    }
                }
            },
        };
        let injector = Arc::new(PacketLossInjector::new(packet_loss_options));
        Self { injector, direction: settings.direction }
    }
}

#[async_trait]
impl ProxyPlugin for PacketLossFaultPlugin {
    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn process_request(
        &self,
        req: ReqwestRequest,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError> {
        Ok(req)
    }

    async fn process_response(
        &self,
        resp: http::Response<Vec<u8>>,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError> {
        Ok(resp)
    }

    async fn process_connect_request(
        &self,
        req: ConnectRequest,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ConnectRequest, ProxyError> {
        // Implement as needed
        Ok(req)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<(), ProxyError> {
        // Implement as needed
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
        let injected_client =
            self.injector.inject(client_stream, &self.direction, event.clone());
        let injected_server =
            self.injector.inject(server_stream, &self.direction, event.clone());

        Ok((injected_client, injected_server))
    }
}

pub fn create_packet_loss_plugin(
    settings: PacketLossSettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(PacketLossFaultPlugin::new_from_settings(settings))
}
