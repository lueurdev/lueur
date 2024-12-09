use async_trait::async_trait;
use reqwest::ClientBuilder as ReqwestClientBuilder;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;

pub mod bandwidth;
pub mod dns;
pub mod jitter;
pub mod latency;
pub mod packet_loss;

use std::marker::Unpin;

use tokio::io::AsyncRead as TokioAsyncRead;
use tokio::io::AsyncWrite as TokioAsyncWrite;

use crate::errors::ProxyError;

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
    ) -> Box<dyn Bidirectional + 'static>;

    async fn apply_on_request_builder(
        &self,
        builder: ReqwestClientBuilder,
    ) -> Result<ReqwestClientBuilder, ProxyError>;

    async fn apply_on_request(
        &self,
        request: ReqwestRequest,
    ) -> Result<ReqwestRequest, ProxyError>;

    async fn apply_on_response(
        &self,
        resp: ReqwestResponse,
    ) -> Result<ReqwestResponse, ProxyError>;
}
