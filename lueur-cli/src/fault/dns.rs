use std::net::SocketAddr;
use std::sync::Arc;

use axum::async_trait;
use axum::http;
use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::*;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use reqwest::dns::Addrs;
use reqwest::dns::Name;
use reqwest::dns::Resolve;
use reqwest::dns::Resolving;
use tokio::sync::RwLock;

use super::Bidirectional;
use super::FaultInjector;
use crate::event::FaultEvent;
use crate::event::ProxyTaskEvent;
use crate::types::Direction;

#[derive(Debug, Clone)]
pub enum DnsStrategy {
    Fixed { rate: f64 },
}

/// DNS Issue Options
#[derive(Clone, Debug)]
pub struct DnsOptions {
    pub strategy: DnsStrategy,
}

/// Custom DNS Resolver that simulates DNS failures
#[derive(Clone, Debug)]
pub struct FaultyResolverInjector {
    inner: Arc<RwLock<TokioAsyncResolver>>,
    options: DnsOptions,
    event: Option<Box<dyn ProxyTaskEvent>>,
}

impl FaultyResolverInjector {
    pub fn new(options: DnsOptions) -> Self {
        let resolver = TokioAsyncResolver::tokio(
            ResolverConfig::default(),
            ResolverOpts::default(),
        );
        Self { options, inner: Arc::new(RwLock::new(resolver)), event: None }
    }

    fn should_apply_fault_resolver(&self) -> bool {
        let mut rng: SmallRng = SmallRng::from_entropy();
        match &self.options.strategy {
            DnsStrategy::Fixed { rate, .. } => rng.gen_bool(*rate),
        }
    }

    pub fn with_event(&mut self, event: Box<dyn ProxyTaskEvent>) {
        self.event = Some(event);
    }
}

impl Resolve for FaultyResolverInjector {
    fn resolve(&self, hostname: Name) -> Resolving {
        let self_clone = self.clone();

        Box::pin(async move {
            let host = hostname.as_str();
            let apply_fault = self_clone.should_apply_fault_resolver();
            tracing::info!("Apply a dns resolver {}", apply_fault);

            if apply_fault {
                let _ = match self_clone.event {
                    Some(event) => {
                        event.with_fault(FaultEvent::Dns { triggered: true })
                    }
                    None => Ok(()),
                };
                let io_error = std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Simulated DNS resolution failure",
                );
                return Err(io_error.into());
            }

            let _ = match self_clone.event {
                Some(event) => {
                    event.with_fault(FaultEvent::Dns { triggered: false })
                }
                None => Ok(()),
            };

            let resolver = self_clone.inner.read().await;
            let lookup = resolver.lookup_ip(host).await?;
            let ips = lookup.into_iter().collect::<Vec<_>>();
            let addrs: Addrs =
                Box::new(ips.into_iter().map(|addr| SocketAddr::new(addr, 0)));

            Ok(addrs)
        })
    }
}

#[async_trait]
impl FaultInjector for FaultyResolverInjector {
    /// Injects latency into a bidirectional stream.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        direction: &Direction,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static> {
        stream
    }

    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, crate::errors::ProxyError> {
        Ok(resp)
    }

    async fn apply_on_request_builder(
        &self,
        builder: reqwest::ClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, crate::errors::ProxyError> {
        let mut cloned = self.clone();
        cloned.with_event(event);

        let resolver: Arc<FaultyResolverInjector> = Arc::new(cloned);
        tracing::debug!("Adding faulty dns resolver on builder");
        let builder = builder.dns_resolver(resolver);
        Ok(builder)
    }

    async fn apply_on_request(
        &self,
        request: reqwest::Request,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::Request, crate::errors::ProxyError> {
        Ok(request)
    }
}
