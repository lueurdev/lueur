use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use async_std_resolver::config;
use async_std_resolver::resolver;
use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::*;
use local_ip_address::local_ip;
use reqwest::dns::Addrs;
use reqwest::dns::Resolve;
use reqwest::dns::Resolving;
use tokio::sync::Mutex;
use tokio::sync::RwLock;

use crate::reporting::DnsTiming;

/// Custom DNS Resolver that measures DNS resolution time and records it.
#[derive(Clone, Debug)]
pub struct TimingResolver {
    resolver: Arc<RwLock<TokioAsyncResolver>>,
    timing: Arc<Mutex<DnsTiming>>,
}

impl TimingResolver {
    /// Creates a new `TimingResolver` with the given report.
    pub fn new(timing: Arc<Mutex<DnsTiming>>) -> Self {
        // Initialize the resolver with default system configuration.
        let resolver = TokioAsyncResolver::tokio(
            ResolverConfig::default(),
            ResolverOpts::default(),
        );

        TimingResolver { resolver: Arc::new(RwLock::new(resolver)), timing }
    }
}

impl Resolve for TimingResolver {
    fn resolve(&self, hostname: reqwest::dns::Name) -> Resolving {
        let self_clone = self.clone();
        let timing = self.timing.clone();

        Box::pin(async move {
            let host = hostname.as_str();
            let resolver = self_clone.resolver.read().await;
            let start_time = Instant::now();
            let lookup = resolver.lookup_ip(host).await?;
            let duration = start_time.elapsed().as_secs_f64();
            {
                let mut timing_lock = timing.lock().await;
                timing_lock.host = host.to_string();
                timing_lock.duration = duration;
            }
            let ips = lookup.into_iter().collect::<Vec<_>>();
            let addrs: Addrs =
                Box::new(ips.into_iter().map(|addr| SocketAddr::new(addr, 0)));

            Ok(addrs)
        })
    }
}

pub async fn lookup_host_address(host: &str) -> String {
    let resolver = resolver(
        config::ResolverConfig::default(),
        config::ResolverOpts::default(),
    )
    .await;

    let response = resolver.lookup_ip(host).await.unwrap();
    let filtered = response
        .into_iter()
        .filter(|addr| {
            tracing::debug!("Addr {:?}", addr);

            match addr {
                IpAddr::V4(ip) => !ip.is_loopback(),
                IpAddr::V6(ip) => !ip.is_loopback(),
            }
        })
        .collect::<Vec<_>>();

    tracing::debug!("Found addresses {:?}", filtered);

    filtered[0].to_string()
}

pub fn map_localhost_to_nic() -> String {
    local_ip().unwrap().to_string()
}
