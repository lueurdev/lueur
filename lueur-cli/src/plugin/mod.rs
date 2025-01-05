use std::fmt;

use axum::async_trait;
use axum::http;
use reqwest::ClientBuilder;
use reqwest::Request as ReqwestRequest;
use serde::Deserialize;
use serde::Serialize;

use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
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
pub trait ProxyPlugin: Send + Sync + std::fmt::Debug + fmt::Display {
    /// Adjust the client builder for forward request proxying
    async fn prepare_client(
        &self,
        builder: ClientBuilder,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ClientBuilder, ProxyError>;

    /// Processes and potentially modifies an outgoing Reqwest HTTP request.
    async fn process_request(
        &self,
        req: ReqwestRequest,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError>;

    /// Processes and potentially modifies an incoming Reqwest HTTP response.
    async fn process_response(
        &self,
        resp: http::Response<Vec<u8>>,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError>;

    /// Processes and potentially modifies a CONNECT request.
    async fn process_connect_request(
        &self,
        req: ConnectRequest,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ConnectRequest, ProxyError>;

    /// Processes the CONNECT response outcome.
    async fn process_connect_response(
        &self,
        success: bool,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<(), ProxyError>;

    async fn inject_tunnel_faults(
        &self,
        client_stream: Box<dyn Bidirectional + 'static>,
        server_stream: Box<dyn Bidirectional + 'static>,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<
        (Box<dyn Bidirectional + 'static>, Box<dyn Bidirectional + 'static>),
        ProxyError,
    >;
}
