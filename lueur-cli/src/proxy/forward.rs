use std::sync::Arc;

use axum::body::Body;
use axum::body::to_bytes;
use axum::http::HeaderMap as AxumHeaderMap;
use axum::http::Request as AxumRequest;
use axum::http::Response as AxumResponse;
use reqwest::header::HeaderMap as ReqwestHeaderMap;
use tracing::error;
use tracing::info;
use url::Url;

use super::ProxyState;
use crate::errors::ProxyError;

/// Converts Axum's HeaderMap to Reqwest's HeaderMap.
fn convert_headers_to_reqwest(
    axum_headers: &AxumHeaderMap,
) -> ReqwestHeaderMap {
    let mut reqwest_headers = ReqwestHeaderMap::new();
    for (key, value) in axum_headers.iter() {
        // Optionally filter out headers like Host if needed
        reqwest_headers.insert(key.clone(), value.clone());
    }
    reqwest_headers
}

/// Converts Reqwest's HeaderMap to Axum's HeaderMap.
fn convert_headers_to_axum(
    reqwest_headers: &ReqwestHeaderMap,
) -> AxumHeaderMap {
    let mut axum_headers = AxumHeaderMap::new();
    for (key, value) in reqwest_headers.iter() {
        axum_headers.insert(key.clone(), value.clone());
    }
    axum_headers
}

pub async fn handle_request(
    req: AxumRequest<Body>,
    state: Arc<ProxyState>,
    upstream: Url,
    passthrough: bool,
) -> Result<AxumResponse<Body>, ProxyError> {
    let forward = Forward::new(state.clone());
    forward.execute(req, upstream, passthrough).await
}

/// Struct responsible for forwarding requests.
pub struct Forward {
    // Shared plugins loaded into the proxy
    state: Arc<ProxyState>,
}

impl Forward {
    /// Creates a new instance of `Forward`.
    pub fn new(state: Arc<ProxyState>) -> Self {
        Self { state }
    }

    /// Executes an Axum request by forwarding it to the target server using
    /// Reqwest.
    ///
    /// Applies plugins after request conversion and after receiving the
    /// response.
    pub async fn execute(
        &self,
        request: AxumRequest<Body>,
        upstream: Url,
        passthrough: bool,
    ) -> Result<AxumResponse<Body>, ProxyError> {
        let method = request.method().clone();
        let headers = request.headers().clone();

        // Clone the plugins Arc to move into the async block
        let plugins = self.state.plugins.clone();

        // Extract the request body as bytes
        let body_bytes =
            to_bytes(request.into_body(), usize::MAX).await.map_err(|e| {
                error!("Failed to read request body: {}", e);
                ProxyError::Internal(format!(
                    "Failed to read request body: {}",
                    e
                ))
            })?;

        let mut client_builder = reqwest::Client::builder();

        if !passthrough {
            {
                let plugins_lock = plugins.read().await;
                for plugin in plugins_lock.iter() {
                    client_builder =
                        plugin.prepare_client(client_builder).await?;
                }
            }
        }

        let client = client_builder.build().unwrap();

        tracing::info!("UPSTREAM: {}", upstream);
        // Build the Reqwest request builder
        let req_builder = client
            .request(method.clone(), upstream)
            .headers(convert_headers_to_reqwest(&headers))
            .body(body_bytes.to_vec());

        let mut upstream_req = req_builder.build().map_err(|e| {
            ProxyError::Internal(format!(
                "Failed to build reqwest request: {}",
                e
            ))
        })?;

        if !passthrough {
            {
                let plugins_lock = plugins.read().await;
                for plugin in plugins_lock.iter() {
                    upstream_req = plugin
                        .process_request(upstream_req)
                        .await
                        .map_err(|e| {
                            error!("Plugin processing request failed: {}", e);
                            e
                        })
                        .unwrap();
                }
            }
        }

        // Execute the Reqwest request
        let response = match client.execute(upstream_req).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to execute reqwest request: {}", e);
                return Err(ProxyError::Internal(format!(
                    "Failed to execute reqwest request: {}",
                    e
                )));
            }
        };

        info!("Received response with status: {}", response.status());

        let mut modified_response = response;

        if !passthrough {
            modified_response = {
                let plugins_lock = plugins.read().await;
                let mut resp = modified_response;
                for plugin in plugins_lock.iter() {
                    resp = plugin.process_response(resp).await?;
                }
                resp
            };
        }

        // Extract the response status, headers, and body
        let status = modified_response.status();
        let resp_headers = modified_response.headers().clone();
        let resp_body_bytes = modified_response.bytes().await.map_err(|e| {
            error!("Failed to read response body: {}", e);
            ProxyError::Internal(format!("Failed to read response body: {}", e))
        })?;

        // Build the Axum response
        let axum_response: AxumResponse<Body> = AxumResponse::default();
        let (mut parts, _) = axum_response.into_parts();

        parts.status = status;
        parts.headers = convert_headers_to_axum(&resp_headers);

        let axum_response = AxumResponse::from_parts(
            parts,
            Body::from(resp_body_bytes.to_vec()),
        );

        info!("Successfully forwarded response with status: {}", status);

        Ok(axum_response)
    }
}
