use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http;

use crate::config::BandwidthSettings;
use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::bandwidth::BandwidthLimitFaultInjector;
use crate::fault::bandwidth::BandwidthOptions;
use crate::fault::bandwidth::BandwidthStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::Direction;

/// RateLimitFaultPlugin that injects rate limiting into tunnel streams.
#[derive(Debug)]
pub struct BandwidthPlugin {
    injector: Arc<dyn FaultInjector>,
    direction: Direction,
}

impl fmt::Display for BandwidthPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bandwidth Plugin")
    }
}

impl BandwidthPlugin {
    pub fn new_from_settings(settings: BandwidthSettings) -> Self {
        let options = BandwidthOptions {
            strategy: BandwidthStrategy::Default {
                bps: settings.bandwidth_rate as usize,
            },
        };
        let injector = Arc::new(BandwidthLimitFaultInjector::new(options));
        Self { injector, direction: settings.direction }
    }
}

#[async_trait]
impl ProxyPlugin for BandwidthPlugin {
    /// Example placeholder implementation. Replace with actual methods as
    /// needed.
    async fn process_connect_request(
        &self,
        _request: crate::types::ConnectRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<crate::types::ConnectRequest, ProxyError> {
        // No modification to ConnectRequest
        Ok(_request)
    }

    async fn process_connect_response(
        &self,
        _success: bool,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<(), ProxyError> {
        // No action on connect response
        Ok(())
    }

    async fn prepare_client(
        &self,
        builder: reqwest::ClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn process_request(
        &self,
        req: reqwest::Request,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::Request, ProxyError> {
        match self.direction.is_ingress() {
            true => self.injector.apply_on_request(req, event).await,
            false => Ok(req),
        }
    }

    async fn process_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError> {
        tracing::debug!("Injecting bandwidth limiter on response");
        self.injector.apply_on_response(resp, event).await
    }

    /// Injects rate limiting into the CONNECT tunnel streams.
    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    > {
        // Inject rate limiting into both client and server streams
        let injected_client =
            self.injector.inject(client_stream, &self.direction, event.clone());
        let injected_server =
            self.injector.inject(server_stream, &self.direction, event.clone());

        Ok((injected_client, injected_server))
    }
}

pub fn create_bandwidth_plugin(
    settings: BandwidthSettings,
) -> Arc<dyn ProxyPlugin> {
    Arc::new(BandwidthPlugin::new_from_settings(settings))
}
