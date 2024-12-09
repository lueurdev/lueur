use std::sync::Arc;

use axum::body::Body;
use axum::http::Request as AxumRequest;
use axum::http::Response as AxumResponse;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tracing::error;
use url::Url;

use crate::errors::ProxyError;
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
) -> Result<ConnectRequest, ProxyError> {
    let lock = plugins.read().await;
    for plugin in lock.iter() {
        connect_request =
            plugin.process_connect_request(connect_request).await?;
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
) -> Result<(), ProxyError> {
    let lock = plugins.read().await;
    for plugin in lock.iter() {
        plugin.process_connect_response(success).await?;
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
) -> Result<AxumResponse<Body>, ProxyError> {
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
        modified_connect_request =
            process_connect_plugins(modified_connect_request, &plugins).await?;
    }

    let addr = format!(
        "{}:{}",
        modified_connect_request.target_host,
        modified_connect_request.target_port
    );

    // Clone plugins to move into the spawned task
    let plugins = plugins.clone();
    let target_host = modified_connect_request.target_host.clone();
    let target_port = modified_connect_request.target_port;

    tokio::task::spawn(async move {
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                match TcpStream::connect(addr).await {
                    Ok(server_stream) => {
                        // Box the streams as bidirectional without direct
                        // fault injection
                        let client_stream: Box<dyn Bidirectional + 'static> =
                            Box::new(TokioIo::new(upgraded));

                        let server_stream: Box<dyn Bidirectional + 'static> =
                            Box::new(server_stream);

                        // Initialize mutable boxed streams
                        let mut modified_client_stream = client_stream;
                        let mut modified_server_stream = server_stream;

                        if !passthrough {
                            // Acquire a read lock for plugins
                            let plugins_lock = plugins.read().await;

                            // Apply each plugin's fault injection
                            for plugin in plugins_lock.iter() {
                                tracing::info!("Plugin {:?}", &plugin);
                                match plugin
                                    .inject_tunnel_faults(
                                        Box::new(modified_client_stream),
                                        Box::new(modified_server_stream),
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

                            drop(plugins_lock); // Release the read lock early
                        }

                        // Start bidirectional data forwarding
                        match copy_bidirectional(
                            &mut *modified_client_stream,
                            &mut *modified_server_stream,
                        )
                        .await
                        {
                            Ok((bytes_from_client, bytes_to_server)) => {
                                tracing::info!(
                                    "Connection closed. Bytes from client: {}, bytes to server: {}",
                                    bytes_from_client,
                                    bytes_to_server
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Error in bidirectional copy: {}",
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to connect to target {}:{} - {}",
                            target_host, target_port, e
                        );

                        if !passthrough {
                            if let Err(notify_err) =
                                notify_plugins_connect_response(&plugins, false)
                                    .await
                            {
                                error!(
                                    "Failed to notify plugins about failed connection: {}",
                                    notify_err
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Upgrade error: {}", e);
                // Handle upgrade error if necessary
            }
        }
    });

    Ok(AxumResponse::new(Body::empty()))
}
