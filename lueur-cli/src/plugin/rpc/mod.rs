use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tonic::Request;
use tonic::transport::Channel;

use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::plugin::ProxyPlugin;
use crate::types::ConnectRequest;

// Include the generated protobuf code.
pub mod service {
    tonic::include_proto!("service");
}

use service::GetPluginInfoRequest;
use service::GetPluginInfoResponse;
use service::ProcessRequestRequest;
use service::ProcessRequestResponse;
use service::ProcessResponseRequest;
use service::ProcessResponseResponse;
use service::plugin_service_client::PluginServiceClient;

// Struct to hold plugin metadata and client
#[derive(Debug, Clone)]
pub struct RemotePlugin {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub url: Option<String>,
    pub platform: Option<String>,
    pub client: Arc<Mutex<PluginServiceClient<Channel>>>,
}

impl fmt::Display for RemotePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gRPC Plugin:\n\
             ---------------------\n\
             Plugin IP          : {}\n\
             Plugin Version        : {}\n\
             Plugin Author     : {}\n\
             Plugin Url      : {}\n\
             Plugin Platform: {}",
            self.name,
            self.version,
            self.author.clone().unwrap_or("".to_string()),
            self.url.clone().unwrap_or("".to_string()),
            self.platform.clone().unwrap_or("".to_string()),
        )
    }
}

#[derive(Debug, Default)]
pub struct RpcPluginManager {
    // Use RwLock for concurrent read access and exclusive write access
    plugins: Arc<RwLock<Vec<RemotePlugin>>>,
}

impl RpcPluginManager {
    /// Creates a new RpcPluginManager.
    pub fn new() -> Self {
        Self { plugins: Arc::new(RwLock::new(Vec::new())) }
    }

    /// Adds a new gRPC server address to the manager.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address of the gRPC server (e.g., "http://127.0.0.1:50051").
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the plugin was successfully added.
    /// * `Err(ProxyError)` if there was an error connecting or fetching
    ///   metadata.
    pub async fn add_plugin_server(
        &self,
        addr: String,
    ) -> Result<(), ProxyError> {
        tracing::debug!("Connecting to gRPC plugins on {}", addr);

        // Establish a connection to the gRPC server
        let mut client =
            PluginServiceClient::connect(addr.clone()).await.map_err(|e| {
                ProxyError::RpcConnectionError(addr.clone(), e.to_string())
            })?;

        // Fetch plugin metadata
        let request = Request::new(GetPluginInfoRequest {});
        let response = client.get_plugin_info(request).await.map_err(|e| {
            ProxyError::RpcCallError(
                addr.clone(),
                "GetPluginInfo".to_string(),
                e.to_string(),
            )
        })?;

        let info: GetPluginInfoResponse = response.into_inner();

        let client = Arc::new(Mutex::new(client));

        tracing::info!("hello");
        let plugin = RemotePlugin {
            name: info.name,
            version: info.version,
            author: if info.author.is_empty() {
                None
            } else {
                Some(info.author)
            },
            url: if info.url.is_empty() { None } else { Some(info.url) },
            platform: if info.platform.is_empty() {
                None
            } else {
                Some(info.platform)
            },
            client,
        };

        tracing::info!("Loaded gRPC plugin {}", &plugin);

        // Add to the plugins list
        let mut plugins = self.plugins.write().await;
        plugins.push(plugin);

        Ok(())
    }

    /// Lists all connected plugins with their metadata.
    pub async fn list_plugins(&self) -> Vec<RemotePlugin> {
        let plugins = self.plugins.read().await;
        plugins.clone()
    }

    pub async fn process_request(
        &self,
        req: reqwest::Request,
    ) -> Result<reqwest::Request, ProxyError> {
        // Extract method, URL, and headers from the request before consuming
        // the body.
        let method = req.method().clone();
        let url = req.url().clone();
        let headers = req.headers().clone();

        // Consume the request to access its body.
        let body = req.body().unwrap();

        // Fully read the request body into memory as bytes.
        // `body.bytes()` reads the entire body and returns `Bytes`.
        let body_bytes = body.as_bytes().unwrap();

        // Convert the Bytes into a Vec<u8> for passing to the plugins
        let mut modified_request = body_bytes.to_vec();

        // Lock and read the list of plugins
        let plugins = self.plugins.read().await;

        // For each plugin, send the current `modified_request` bytes and get
        // back updated bytes.
        for plugin in plugins.iter() {
            let req =
                ProcessRequestRequest { request: modified_request.clone() };

            let mut client = plugin.client.lock().await;
            let response = client
                .process_request(tonic::Request::new(req))
                .await
                .map_err(|e| {
                    ProxyError::RpcCallError(
                        plugin.name.clone(),
                        "ProcessRequest".to_string(),
                        e.to_string(),
                    )
                })?;

            let resp: ProcessRequestResponse = response.into_inner();
            modified_request = resp.modified_request;
        }

        // Rebuild a new `reqwest::Request` using the same method, URL, and
        // headers. Assign the modified body as the request body.
        let mut new_req = reqwest::Request::new(method, url);
        *new_req.headers_mut() = headers;
        *new_req.body_mut() = Some(reqwest::Body::from(modified_request));

        Ok(new_req)
    }

    /// Processes a response by forwarding it to all plugins sequentially.
    ///
    /// # Arguments
    ///
    /// * `response` - The original response bytes.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<u8>, ProxyError>` - The modified response after
    ///   processing.
    pub async fn process_response(
        &self,
        response: Vec<u8>,
    ) -> Result<Vec<u8>, ProxyError> {
        let plugins = self.plugins.read().await;
        let mut modified_response = response;

        for plugin in plugins.iter() {
            let req =
                ProcessResponseRequest { response: modified_response.clone() };

            let mut client = plugin.client.lock().await;

            let response = client
                .process_response(Request::new(req))
                .await
                .map_err(|e| {
                    ProxyError::RpcCallError(
                        plugin.name.clone(),
                        "ProcessResponse".to_string(),
                        e.to_string(),
                    )
                })?;
            let resp: ProcessResponseResponse = response.into_inner();
            modified_response = resp.modified_response;
        }

        Ok(modified_response)
    }
}

#[derive(Debug)]
pub struct RpcPlugin {
    manager: Arc<RpcPluginManager>,
}

impl RpcPlugin {
    pub async fn new(addresses: Vec<String>) -> Self {
        let manager = RpcPluginManager::new();

        for addr in addresses {
            let _ = manager.add_plugin_server(addr.clone()).await;
        }

        Self { manager: Arc::new(manager) }
    }
}

#[async_trait]
impl ProxyPlugin for RpcPlugin {
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
        self.manager.process_request(req).await
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
        Ok((client_stream, server_stream))
    }
}

/// Factory function to create a dns plugin
pub async fn load_remote_plugins(
    addresses: Vec<String>,
) -> Arc<dyn ProxyPlugin> {
    tracing::debug!("Loading gRPC plugins");
    Arc::new(RpcPlugin::new(addresses).await)
}
