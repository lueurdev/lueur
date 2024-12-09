use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::async_trait;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

use super::Bidirectional;
use super::FaultInjector;

/// Enumeration of packet loss strategies.
#[derive(Debug, Clone)]
pub enum PacketLossStrategy {
    Bernoulli { loss_probability: f64 },
    GilbertElliott { loss_probability: f64 },
}

/// Configuration for packet loss injection.
#[derive(Debug, Clone)]
pub struct PacketLossOptions {
    pub strategy: PacketLossStrategy,
}

/// PacketLossInjector that randomly drops read and write operations based on
/// the configured probability.
#[derive(Debug)]
pub struct PacketLossInjector {
    options: PacketLossOptions,
}

impl PacketLossInjector {
    pub fn new(options: PacketLossOptions) -> Self {
        Self { options }
    }

    /// Determines whether to drop the current packet based on the loss
    /// probability.
    fn should_drop(&self, rng: &mut SmallRng) -> bool {
        match &self.options.strategy {
            PacketLossStrategy::Bernoulli { loss_probability, .. } => {
                rng.r#gen::<f64>() < *loss_probability
            }
            PacketLossStrategy::GilbertElliott { loss_probability, .. } => {
                rng.r#gen::<f64>() < *loss_probability
            }
        }
    }
}

impl Clone for PacketLossInjector {
    fn clone(&self) -> Self {
        Self { options: self.options.clone() }
    }
}

#[async_trait]
impl FaultInjector for PacketLossInjector {
    /// Injects packet loss into a bidirectional stream.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
    ) -> Box<dyn Bidirectional + 'static> {
        Box::new(PacketLossStream::new(stream, self.clone()))
    }

    async fn apply_on_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<reqwest::Response, crate::errors::ProxyError> {
        Ok(resp)
    }

    async fn apply_on_request_builder(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, crate::errors::ProxyError> {
        Ok(builder)
    }

    async fn apply_on_request(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Request, crate::errors::ProxyError> {
        Ok(request)
    }
}

/// A wrapper around a bidirectional stream that injects packet loss.
pub struct PacketLossStream {
    stream: Box<dyn Bidirectional + 'static>,
    injector: PacketLossInjector,
    rng: SmallRng,
}

impl PacketLossStream {
    /// Creates a new PacketLossStream.
    fn new(
        stream: Box<dyn Bidirectional + 'static>,
        injector: PacketLossInjector,
    ) -> Self {
        Self { stream, injector, rng: SmallRng::from_entropy() }
    }

    /// Determines whether to drop the current packet.
    fn should_drop(&mut self) -> bool {
        self.injector.should_drop(&mut self.rng)
    }
}

impl AsyncRead for PacketLossStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.should_drop() {
            // Simulate packet loss by not reading any data.
            // Alternatively, you could fill the buffer with zeros or keep it
            // pending. Here, we'll skip reading and return
            // immediately as if no data was read.
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }
}

impl AsyncWrite for PacketLossStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.should_drop() {
            // Simulate packet loss by not writing the data.
            // Return as if all bytes were written successfully.
            Poll::Ready(Ok(buf.len()))
        } else {
            Pin::new(&mut self.stream).poll_write(cx, buf)
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Optionally, inject packet loss into flush operations.
        // For simplicity, we'll delegate flush without injecting packet loss.
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Optionally, inject packet loss into shutdown operations.
        // For simplicity, we'll delegate shutdown without injecting packet
        // loss.
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
