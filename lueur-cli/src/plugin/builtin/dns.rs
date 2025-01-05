use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

use crate::config::DnsSettings;
use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::dns::DnsOptions;
use crate::fault::dns::DnsStrategy;
use crate::fault::dns::FaultyResolverInjector;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::Direction;

#[derive(Debug)]
pub struct DnsFaultPlugin {
    injector: Arc<dyn FaultInjector>,
    direction: Direction,
}

impl fmt::Display for DnsFaultPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DNS Plugin")
    }
}

impl DnsFaultPlugin {
    pub fn new_from_settings(settings: DnsSettings) -> Self {
        let dns_options = DnsOptions {
            strategy: DnsStrategy::Fixed {
                rate: settings.dns_rate as f64 / 100.0,
            },
        };
        let injector = Arc::new(FaultyResolverInjector::new(dns_options));
        Self { injector, direction: settings.direction }
    }
}

#[async_trait]
impl ProxyPlugin for DnsFaultPlugin {
    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        tracing::debug!("Applying DNS fault resolver");
        self.injector.apply_on_request_builder(builder, event).await
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
        Ok(resp)
    }

    async fn process_connect_request(
        &self,
        req: ConnectRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ConnectRequest, ProxyError> {
        // Implement as needed
        Ok(req)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
        event: Box<dyn ProxyTaskEvent>,
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

/// Factory function to create a dns plugin
pub fn create_dns_plugin(settings: DnsSettings) -> Arc<dyn ProxyPlugin> {
    Arc::new(DnsFaultPlugin::new_from_settings(settings))
}
