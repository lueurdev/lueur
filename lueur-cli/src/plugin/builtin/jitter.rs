use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http;
use reqwest::Request as ReqwestRequest;

use crate::config::JitterSettings;
use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::jitter::JitterInjector;
use crate::fault::jitter::JitterOptions;
use crate::fault::jitter::JitterStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;
use crate::types::Direction;

/// JitterFaultPlugin is a plugin that injects jitter into streams using
/// amplitude and frequency.
#[derive(Debug)]
pub struct JitterFaultPlugin {
    injector: Arc<dyn FaultInjector>,
    direction: Direction,
}

impl fmt::Display for JitterFaultPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Jitter Plugin")
    }
}

impl JitterFaultPlugin {
    pub fn new_from_settings(settings: JitterSettings) -> Self {
        let jitter_options = JitterOptions {
            strategy: JitterStrategy::Default {
                amplitude: Duration::from_secs_f64(settings.jitter_amplitude),
                frequency: settings.jitter_frequency,
            },
        };
        let injector = Arc::new(JitterInjector::new(jitter_options));
        Self { injector, direction: settings.direction }
    }
}

#[async_trait]
impl ProxyPlugin for JitterFaultPlugin {
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

/// Factory function to create a jitter fault plugin.
pub fn create_jitter_plugin(settings: JitterSettings) -> Arc<dyn ProxyPlugin> {
    Arc::new(JitterFaultPlugin::new_from_settings(settings))
}
