use axum::async_trait;
use reqwest::ClientBuilder;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;
use serde::Deserialize;
use serde::Serialize;

use crate::errors::ProxyError;
use crate::fault::Bidirectional;
use crate::types::ConnectRequest;

pub(crate) mod builtin;
pub(crate) mod rpc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePluginInfo {
    name: String,
    version: String,
    author: String,
    url: String,
}

#[async_trait]
pub trait ProxyPlugin: Send + Sync + std::fmt::Debug {
    /// Adjust the client builder for forward request proxying
    async fn prepare_client(
        &self,
        builder: ClientBuilder,
    ) -> Result<ClientBuilder, ProxyError>;

    /// Processes and potentially modifies an outgoing Reqwest HTTP request.
    async fn process_request(
        &self,
        req: ReqwestRequest,
    ) -> Result<ReqwestRequest, ProxyError>;

    /// Processes and potentially modifies an incoming Reqwest HTTP response.
    async fn process_response(
        &self,
        resp: ReqwestResponse,
    ) -> Result<ReqwestResponse, ProxyError>;

    /// Processes and potentially modifies a CONNECT request.
    async fn process_connect_request(
        &self,
        req: ConnectRequest,
    ) -> Result<ConnectRequest, ProxyError>;

    /// Processes the CONNECT response outcome.
    async fn process_connect_response(
        &self,
        success: bool,
    ) -> Result<(), ProxyError>;

    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    >;
}
