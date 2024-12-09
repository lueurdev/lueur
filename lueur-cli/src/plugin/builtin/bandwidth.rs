use std::sync::Arc;

use async_trait::async_trait;

use crate::config::BandwidthSettings;
use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::bandwidth::BandwidthLimitFaultInjector;
use crate::fault::bandwidth::BandwidthOptions;
use crate::fault::bandwidth::BandwidthStrategy;
use crate::plugin::ProxyPlugin;

/// RateLimitFaultPlugin that injects rate limiting into tunnel streams.
#[derive(Debug)]
pub struct BandwidthPlugin {
    injector: Arc<dyn FaultInjector>,
}

impl BandwidthPlugin {
    pub fn new_from_settings(settings: BandwidthSettings) -> Self {
        let options = BandwidthOptions {
            strategy: BandwidthStrategy::Default {
                bps: settings.bandwidth_rate as usize,
            },
        };
        let injector = Arc::new(BandwidthLimitFaultInjector::new(options));
        Self { injector }
    }
}

#[async_trait]
impl ProxyPlugin for BandwidthPlugin {
    /// Example placeholder implementation. Replace with actual methods as
    /// needed.
    async fn process_connect_request(
        &self,
        _request: crate::types::ConnectRequest,
    ) -> Result<crate::types::ConnectRequest, ProxyError> {
        // No modification to ConnectRequest
        Ok(_request)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
    ) -> Result<(), ProxyError> {
        // No action on connect response
        Ok(())
    }

    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn process_request(
        &self,
        req: reqwest::Request,
    ) -> Result<reqwest::Request, ProxyError> {
        tracing::debug!("Injecting bandwidth limiter on request");
        self.injector.apply_on_request(req).await
    }

    async fn process_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<reqwest::Response, ProxyError> {
        tracing::debug!("Injecting bandwidth limiter on response");
        self.injector.apply_on_response(resp).await
    }

    /// Injects rate limiting into the CONNECT tunnel streams.
    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    > {
        // Inject rate limiting into both client and server streams
        let injected_client = self.injector.inject(client_stream);
        let injected_server = self.injector.inject(server_stream);

        Ok((injected_client, injected_server))
    }
}

pub fn create_bandwidth_plugin(
    settings: BandwidthSettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(BandwidthPlugin::new_from_settings(settings))
}
