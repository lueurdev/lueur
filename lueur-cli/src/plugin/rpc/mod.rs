use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tonic::Request;
use tonic::transport::Channel;

use crate::errors::ProxyError;

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

#[derive(Debug, Default)]
pub struct RpcPluginManager {
    // Use RwLock for concurrent read access and exclusive write access
    plugins: Arc<RwLock<Vec<RemotePlugin>>>,
}

impl RpcPluginManager {
    /// Creates a new RpcPluginManager.
    pub fn _new() -> Self {
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
    pub async fn _add_plugin_server(
        &self,
        addr: String,
    ) -> Result<(), ProxyError> {
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

        // Add to the plugins list
        let mut plugins = self.plugins.write().await;
        plugins.push(plugin);

        Ok(())
    }

    /// Lists all connected plugins with their metadata.
    pub async fn _list_plugins(&self) -> Vec<RemotePlugin> {
        let plugins = self.plugins.read().await;
        plugins.clone()
    }

    /// Processes a request by forwarding it to all plugins sequentially.
    ///
    /// # Arguments
    ///
    /// * `request` - The original request bytes.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<u8>, ProxyError>` - The modified request after processing.
    pub async fn _process_request(
        &self,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ProxyError> {
        let plugins = self.plugins.read().await;
        let mut modified_request = request;

        for plugin in plugins.iter() {
            let req =
                ProcessRequestRequest { request: modified_request.clone() };

            let mut client = plugin.client.lock().await;
            let response = client
                .process_request(Request::new(req))
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

        Ok(modified_request)
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
    pub async fn _process_response(
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
