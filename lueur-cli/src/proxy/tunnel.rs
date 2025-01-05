use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;

use async_std_resolver::config;
use async_std_resolver::resolver;
use axum::body::Body;
use axum::http::Request as AxumRequest;
use axum::http::Response as AxumResponse;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tracing::error;
use url::Url;

use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::fault::Bidirectional;
use crate::plugin::ProxyPlugin;
use crate::proxy::ProxyState;
use crate::types::ConnectRequest;

/// Processes the CONNECT request through all available plugins.
///
/// # Arguments
///
/// * `connect_request` - The CONNECT request details.
/// * `plugins` - A list of plugins.
///
/// # Returns
///
/// * `Result<ConnectRequest, ProxyError>` - The modified CONNECT request or a
///   ProxyError.
async fn process_connect_plugins(
    mut connect_request: ConnectRequest,
    plugins: &Arc<tokio::sync::RwLock<Vec<Arc<dyn ProxyPlugin>>>>,
    event: Box<dyn ProxyTaskEvent>,
) -> Result<ConnectRequest, ProxyError> {
    let lock = plugins.read().await;
    for plugin in lock.iter() {
        connect_request = plugin
            .process_connect_request(connect_request, event.clone())
            .await?;
    }
    Ok(connect_request)
}

/// Notifies plugins about the CONNECT response outcome.
///
/// # Arguments
///
/// * `plugins` - A list of plugins.
/// * `success` - Whether the CONNECT operation was successful.
///
/// # Returns
///
/// * `Result<(), ProxyError>` - `Ok` if notifications were successful, or a
///   ProxyError.
async fn notify_plugins_connect_response(
    plugins: &Arc<tokio::sync::RwLock<Vec<Arc<dyn ProxyPlugin>>>>,
    success: bool,
    event: Box<dyn ProxyTaskEvent>,
) -> Result<(), ProxyError> {
    let lock = plugins.read().await;
    for plugin in lock.iter() {
        plugin.process_connect_response(success, event.clone()).await?;
    }
    Ok(())
}

/// Handles CONNECT method requests by establishing a TCP tunnel,
/// injecting any configured network faults, and applying plugin middleware.
pub async fn handle_connect(
    req: AxumRequest<Body>,
    app_state: Arc<ProxyState>,
    upstream: Url,
    passthrough: bool,
    event: Box<dyn ProxyTaskEvent>,
) -> Result<AxumResponse<Body>, ProxyError> {
    let upstream_str = upstream.to_string();

    let target_host = upstream.host().unwrap().to_string();
    let target_port = upstream.port_or_known_default().unwrap();

    let connect_request =
        ConnectRequest { target_host: target_host.clone(), target_port };

    // Acquire a read lock for plugins
    let plugins: Arc<tokio::sync::RwLock<Vec<Arc<dyn ProxyPlugin>>>> =
        app_state.plugins.clone();

    let mut modified_connect_request = connect_request;

    if !passthrough {
        // Process the CONNECT request through plugins
        modified_connect_request = process_connect_plugins(
            modified_connect_request,
            &plugins,
            event.clone(),
        )
        .await?;
    }

    let host = modified_connect_request.target_host;

    // Clone plugins to move into the spawned task
    let plugins = plugins.clone();

    let _ = tokio::spawn(async move {
        let event = event.clone();
        let upstream_str = upstream.to_string();

        let port = modified_connect_request.target_port;
        let start = Instant::now();
        let addresses = resolve_addresses(host.clone()).await;

        let _ =
            event.on_resolved(host.clone(), start.elapsed().as_millis_f64());

        let addr: SocketAddr = SocketAddr::new(addresses[0], port);
        tracing::debug!("Connecting to {}", &addr);

        match hyper::upgrade::on(req).await {
            Ok(upgraded) => match TcpStream::connect(addr).await {
                Ok(server_stream) => {
                    tracing::debug!("Tunnel to {} created", &upstream_str);

                    let start = Instant::now();
                    let _ = event.on_started(upstream_str);

                    let client_stream: Box<dyn Bidirectional + 'static> =
                        Box::new(TokioIo::new(upgraded));

                    let server_stream: Box<dyn Bidirectional + 'static> =
                        Box::new(server_stream);

                    let mut modified_client_stream = client_stream;
                    let mut modified_server_stream = server_stream;

                    if !passthrough {
                        let plugins_lock = plugins.read().await;

                        for plugin in plugins_lock.iter() {
                            tracing::debug!("Plugin {:?}", &plugin);
                            match plugin
                                .inject_tunnel_faults(
                                    Box::new(modified_client_stream),
                                    Box::new(modified_server_stream),
                                    event.clone(),
                                )
                                .await
                            {
                                Ok((client, server)) => {
                                    modified_client_stream = client;
                                    modified_server_stream = server;
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Plugin failed to inject tunnel faults: {}",
                                        e
                                    );
                                    return;
                                }
                            }
                        }

                        drop(plugins_lock);
                    }

                    let stream = match copy_bidirectional(
                        &mut *modified_client_stream,
                        &mut *modified_server_stream,
                    )
                    .await
                    {
                        Ok((bytes_from_client, bytes_to_server)) => {
                            let _ = event.on_response(0);
                            let _ = event.on_completed(
                                start.elapsed(),
                                bytes_from_client,
                                bytes_to_server,
                            );
                            tracing::debug!(
                                "Connection closed. Bytes from client: {}, bytes to server: {}",
                                bytes_from_client,
                                bytes_to_server
                            );
                        }
                        Err(e) => {
                            let _ = event.on_response(500);
                            let _ = event.on_completed(start.elapsed(), 0, 0);
                            tracing::error!(
                                "Error in bidirectional copy: {}",
                                e
                            );
                        }
                    };

                    stream
                }
                Err(e) => {
                    error!(
                        "Failed to connect to target {}:{} - {}",
                        target_host, target_port, e
                    );

                    if !passthrough {
                        if let Err(notify_err) =
                            notify_plugins_connect_response(
                                &plugins,
                                false,
                                event.clone(),
                            )
                            .await
                        {
                            error!(
                                "Failed to notify plugins about failed connection: {}",
                                notify_err
                            );
                        }
                    }
                }
            },
            Err(e) => {
                error!("Upgrade error: {}", e);
                // Handle upgrade error if necessary
            }
        }
    });

    tracing::debug!("Responding to tunnel request for {}", &upstream_str);
    Ok(AxumResponse::new(Body::empty()))
}

pub async fn resolve_addresses(host: String) -> Vec<IpAddr> {
    let resolver = resolver(
        config::ResolverConfig::default(),
        config::ResolverOpts::default(),
    )
    .await;

    let response = resolver.lookup_ip(host).await.unwrap();
    let filtered = response.into_iter().collect::<Vec<_>>();

    tracing::debug!("Found addresses {:?}", filtered);

    filtered
}
