use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

use crate::config::DnsSettings;
use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::dns::DnsOptions;
use crate::fault::dns::DnsStrategy;
use crate::fault::dns::FaultyResolverInjector;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;

#[derive(Debug)]
pub struct DnsFaultPlugin {
    injector: Arc<dyn FaultInjector>,
}

impl DnsFaultPlugin {
    pub fn new_from_settings(settings: DnsSettings) -> Self {
        let dns_options = DnsOptions {
            strategy: DnsStrategy::Fixed {
                rate: settings.dns_rate as f64 / 100.0,
            },
        };
        let injector = Arc::new(FaultyResolverInjector::new(dns_options));
        Self { injector }
    }
}

#[async_trait]
impl ProxyPlugin for DnsFaultPlugin {
    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        tracing::debug!("Applying DNS fault resolver");
        self.injector.apply_on_request_builder(builder).await
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

/// Factory function to create a dns plugin
pub fn create_dns_plugin(settings: DnsSettings) -> Arc<dyn ProxyPlugin> {
    Arc::new(DnsFaultPlugin::new_from_settings(settings))
}
