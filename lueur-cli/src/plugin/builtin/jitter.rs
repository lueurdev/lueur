use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

use crate::config::JitterSettings;
use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::fault::FaultInjector;
use crate::fault::jitter::JitterInjector;
use crate::fault::jitter::JitterOptions;
use crate::fault::jitter::JitterStrategy;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;

/// JitterFaultPlugin is a plugin that injects jitter into streams using
/// amplitude and frequency.
#[derive(Debug)]
pub struct JitterFaultPlugin {
    injector: Arc<dyn FaultInjector>,
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
        Self { injector }
    }
}

#[async_trait]
impl ProxyPlugin for JitterFaultPlugin {
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

/// Factory function to create a jitter fault plugin.
pub fn create_jitter_plugin(settings: JitterSettings) -> Arc<dyn ProxyPlugin> {
    Arc::new(JitterFaultPlugin::new_from_settings(settings))
}
