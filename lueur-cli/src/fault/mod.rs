use async_trait::async_trait;
use axum::http;
use reqwest::ClientBuilder as ReqwestClientBuilder;
use reqwest::Request as ReqwestRequest;

pub mod bandwidth;
pub mod dns;
pub mod jitter;
pub mod latency;
pub mod packet_loss;

use std::marker::Unpin;

use tokio::io::AsyncRead as TokioAsyncRead;
use tokio::io::AsyncWrite as TokioAsyncWrite;

use crate::errors::ProxyError;
use crate::event::ProxyTaskEvent;
use crate::types::Direction;

/// A composite trait that combines AsyncRead, AsyncWrite, Unpin, and Send.
pub trait Bidirectional:
    TokioAsyncRead + TokioAsyncWrite + Unpin + Send
{
}

impl<T> Bidirectional for T where
    T: TokioAsyncRead + TokioAsyncWrite + Unpin + Send
{
}

#[async_trait]
pub trait FaultInjector: Send + Sync + std::fmt::Debug {
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        direction: &Direction,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static>;

    async fn apply_on_request_builder(
        &self,
        builder: ReqwestClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestClientBuilder, ProxyError>;

    async fn apply_on_request(
        &self,
        request: ReqwestRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError>;

    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError>;
}
