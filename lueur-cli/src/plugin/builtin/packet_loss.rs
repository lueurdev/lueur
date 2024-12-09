use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

use crate::config::PacketLossSettings;
use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::packet_loss::PacketLossInjector;
use crate::fault::packet_loss::PacketLossOptions;
use crate::fault::packet_loss::PacketLossStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::PacketLossType;

/// PacketLossFaultPlugin is a plugin that injects packet loss into streams
/// based on configured settings.
#[derive(Debug)]
pub struct PacketLossFaultPlugin {
    injector: Arc<dyn FaultInjector>,
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
        Self { injector }
    }
}

#[async_trait]
impl ProxyPlugin for PacketLossFaultPlugin {
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
        Ok(resp)
    }

    async fn process_connect_request(
        &self,
        req: ConnectRequest,
    ) -> Result<ConnectRequest, ProxyError> {
        // Implement as needed
        Ok(req)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
    ) -> Result<(), ProxyError> {
        // Implement as needed
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

pub fn create_packet_loss_plugin(
    settings: PacketLossSettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(PacketLossFaultPlugin::new_from_settings(settings))
}
