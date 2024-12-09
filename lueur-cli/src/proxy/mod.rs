pub mod forward;
pub mod tunnel;

use std::net::SocketAddr;
use std::sync::Arc;

use ::oneshot::Sender;
use axum::body::Body;
use axum::http::Request;
use hyper::Method;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tower::Service;
use tower::service_fn;
use url::Url;

use crate::config::ProxyConfig;
use crate::errors::ProxyError;
use crate::plugin::ProxyPlugin;
use crate::plugin::builtin::load_builtin_plugins;
use crate::resolver::map_localhost_to_nic;

/// Shared application state
#[derive(Clone)]
pub struct ProxyState {
    pub plugins: Arc<RwLock<Vec<Arc<dyn ProxyPlugin>>>>,
    pub shared_config: Arc<RwLock<ProxyConfig>>,
    pub upstream_hosts: Arc<RwLock<Vec<String>>>,
    pub stealth: bool,
}

impl ProxyState {
    pub fn new(stealth: bool) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(Vec::new())),
            shared_config: Arc::new(RwLock::new(ProxyConfig::default())),
            upstream_hosts: Arc::new(RwLock::new(Vec::new())),
            stealth: stealth,
        }
    }

    /// Update the plugins
    pub async fn update_plugins(
        &self,
        mut new_plugins: Vec<Arc<dyn ProxyPlugin>>,
    ) {
        let mut plugins = self.plugins.write().await;
        plugins.append(&mut new_plugins);
    }

    /// Update the shared configuration.
    pub async fn update_config(&self, new_config: ProxyConfig) {
        let mut config = self.shared_config.write().await;
        *config = new_config;
    }

    /// Update the upstream hosts.
    pub async fn update_upstream_hosts(&self, new_hosts: Vec<String>) {
        tracing::info!("Allowed hosts {:?}", new_hosts);
        let mut hosts = self.upstream_hosts.write().await;
        *hosts = new_hosts;
    }
}

pub async fn run_proxy(
    proxy_address: String,
    state: Arc<ProxyState>,
    mut shutdown_rx: broadcast::Receiver<()>,
    readiness_tx: Sender<()>,
    mut config_rx: watch::Receiver<ProxyConfig>,
) -> Result<(), ProxyError> {
    let addr: SocketAddr = proxy_address.parse().map_err(|e| {
        ProxyError::Internal(format!(
            "Failed to parse proxy address {}: {}",
            proxy_address, e
        ))
    })?;

    let state_cloned = state.clone();
    let tower_service = service_fn(move |req: Request<Incoming>| {
        let state = state.clone();
        let req = req.map(Body::new);
        async move {
            let state = state.clone();
            let method = req.method().clone();
            let scheme = req.uri().scheme_str().unwrap_or("http").to_string();
            let authority: Option<String> =
                req.uri().authority().map(|a| a.as_str().to_string());
            let host_header = req
                .headers()
                .get(axum::http::header::HOST)
                .map(|v| v.to_str().ok().map(|s| s.to_string()))
                .flatten();

            let path = match req.uri().path_and_query() {
                Some(path) => path.to_string(),
                None => "/".to_string(),
            };

            let upstream = match determine_upstream(
                scheme,
                authority,
                host_header,
                path,
                state.stealth,
            )
            .await
            {
                Ok(url) => url,
                Err(e) => {
                    tracing::error!("Failed to determine upstream: {}", e);
                    return Err(e);
                }
            };

            let mut passthrough = true;

            // Check if host is in the allowed list
            let hosts = state.upstream_hosts.read().await;
            let upstream_host = get_host(&upstream);
            tracing::debug!("Upstream host: {}", upstream_host);
            if hosts.contains(&upstream_host) {
                tracing::debug!("Upstream host in allowed list");
                passthrough = false;
            }

            let upstream_url: Url = upstream.parse().unwrap();

            if method == Method::CONNECT {
                let r = tunnel::handle_connect(
                    req,
                    state.clone(),
                    upstream_url,
                    passthrough,
                )
                .await;
                let resp = r.unwrap();
                Ok(hyper::Response::from(resp))
            } else {
                let r = forward::handle_request(
                    req,
                    state.clone(),
                    upstream_url,
                    passthrough,
                )
                .await;
                // replace with a match
                let resp = r.unwrap();
                Ok(hyper::Response::from(resp))
            }
        }
    });

    let hyper_service =
        hyper::service::service_fn(move |request: Request<Incoming>| {
            tower_service.clone().call(request)
        });

    let state = state_cloned.clone();
    tokio::spawn(async move {
        let state = state.clone();
        while config_rx.changed().await.is_ok() {
            let new_config = config_rx.borrow().clone();
            tracing::info!("Received new configuration: {:?}", new_config);
            state.update_config(new_config).await;
            let new_plugins = load_builtin_plugins(state.shared_config.clone())
                .await
                .unwrap();
            state.update_plugins(new_plugins).await;
        }
        tracing::info!("Configuration update channel closed.");
    });

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        ProxyError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to bind to address {}: {}", addr, e),
        ))
    })?;

    tracing::info!("Proxy listening on {}", addr.to_string());

    tokio::spawn(async move {
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            tracing::info!("Accepted connection from {}", addr);
                            let io = TokioIo::new(stream);
                            let hyper_service = hyper_service.clone();
                            tokio::task::spawn(async move {
                                if let Err(err) = http1::Builder::new()
                                    .preserve_header_case(true)
                                    .title_case_headers(true)
                                    .serve_connection(io, hyper_service)
                                    .with_upgrades()
                                    .await
                                {
                                    tracing::error!("Failed to serve connection: {:?}", err);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Failed to accept connection: {}", e);
                            continue;
                        }
                    }
                },
                _ = shutdown_rx.recv() => {
                    tracing::info!("Shutdown signal received. Stopping listener.");
                    break;
                }
            }
        }
    });

    readiness_tx.send(()).map_err(|e| {
        ProxyError::Internal(format!("Failed to send readiness signal: {}", e))
    })?;

    Ok(())
}

async fn determine_upstream(
    scheme: String,
    authority: Option<String>,
    host_header: Option<String>,
    path: String,
    stealth: bool,
) -> Result<String, ProxyError> {
    let upstream = if let Some(auth) = authority {
        format!("{}://{}{}", scheme, auth, path)
    } else if let Some(host_str) = host_header {
        let (mut host, port) = parse_domain_with_scheme("http", &host_str);
        if stealth && host.as_str() == "localhost" {
            host = map_localhost_to_nic()
        }
        format!("http://{}:{}{}", host, port, path)
    } else {
        return Err(ProxyError::InvalidRequest(
            "Unable to determine upstream target".into(),
        ));
    };

    Ok(upstream)
}

fn parse_domain_with_scheme(scheme: &str, domain: &str) -> (String, String) {
    let default_port = match scheme {
        "https" => "443",
        _ => "80",
    };

    if let Some((host, port_str)) = domain.split_once(':') {
        let port = if port_str.is_empty() { default_port } else { port_str };
        (host.to_string(), port.to_string())
    } else {
        (domain.to_string(), default_port.to_string())
    }
}

fn get_host(upstream_url: &String) -> String {
    let url = Url::parse(upstream_url).unwrap();
    let host = url.host_str().ok_or("Missing host").unwrap().to_string();
    let port = url.port_or_known_default().unwrap();
    format!("{}:{}", host, port)
}
