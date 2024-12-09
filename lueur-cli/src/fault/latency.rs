// src/fault/latency.rs

use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::async_trait;
use pin_project::pin_project;
use rand_distr::Distribution;
use rand_distr::Normal;
use rand_distr::Pareto;
use rand_distr::Uniform;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;
use tokio::time::Duration;
use tokio::time::Sleep;
use tokio::time::sleep;

use super::Bidirectional;
use super::FaultInjector;

#[derive(Debug, Clone)]
pub enum LatencyStrategy {
    Normal { mean: f64, stddev: f64 },
    Pareto { shape: f64, scale: f64 },
    ParetoNormal { shape: f64, scale: f64, mean: f64, stddev: f64 },
    Uniform { min: f64, max: f64 },
}

#[derive(Debug, Clone)]
pub struct LatencyOptions {
    pub strategy: LatencyStrategy,
}

#[derive(Debug)]
pub struct LatencyInjector {
    options: LatencyOptions,
}

impl LatencyInjector {
    pub fn new(options: LatencyOptions) -> Self {
        Self { options }
    }

    fn get_delay(&self) -> Duration {
        let mut rng = rand::thread_rng();
        match &self.options.strategy {
            LatencyStrategy::Normal { mean, stddev } => {
                let normal = Normal::new(*mean, *stddev).unwrap();
                let sample = normal.sample(&mut rng).max(0.0);
                Duration::from_millis(sample as u64)
            }
            LatencyStrategy::Pareto { shape, scale } => {
                let pareto = Pareto::new(*shape, *scale).unwrap();
                let sample = pareto.sample(&mut rng).max(0.0);
                Duration::from_millis(sample as u64)
            }
            LatencyStrategy::ParetoNormal { shape, scale, mean, stddev } => {
                let pareto = Pareto::new(*shape, *scale).unwrap();
                let pareto_sample = pareto.sample(&mut rng).max(0.0);
                let normal = Normal::new(*mean, *stddev).unwrap();
                let normal_sample = normal.sample(&mut rng).max(0.0);
                let total = pareto_sample + normal_sample;
                Duration::from_millis(total as u64)
            }
            LatencyStrategy::Uniform { min, max } => {
                let uniform = Uniform::new(*min, *max);
                let sample = uniform.sample(&mut rng).max(0.0);
                Duration::from_millis(sample as u64)
            }
        }
    }
}

impl Clone for LatencyInjector {
    fn clone(&self) -> Self {
        Self { options: self.options.clone() }
    }
}

#[async_trait]
impl FaultInjector for LatencyInjector {
    /// Injects latency into a bidirectional stream.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
    ) -> Box<dyn Bidirectional + 'static> {
        let delay = self.get_delay();
        tracing::info!("Injecting latency {:?}", delay);
        Box::new(LatencyStream::new(stream, delay))
    }

    async fn apply_on_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<reqwest::Response, crate::errors::ProxyError> {
        let delay = self.get_delay();
        tracing::info!("Adding latency {:?}", delay);
        sleep(delay).await;
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

#[pin_project]
struct LatencyStream {
    #[pin]
    stream: Box<dyn Bidirectional + 'static>,
    #[pin]
    delay: Duration,
    #[pin]
    read_sleep: Option<Pin<Box<Sleep>>>,
    #[pin]
    write_sleep: Option<Pin<Box<Sleep>>>,
}

impl LatencyStream {
    fn new(stream: Box<dyn Bidirectional + 'static>, delay: Duration) -> Self {
        Self { stream, delay, read_sleep: None, write_sleep: None }
    }
}

impl AsyncRead for LatencyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut this = self.project();

        if let Some(sleep_fut) = this.read_sleep.as_mut().as_pin_mut() {
            match sleep_fut.poll(cx) {
                Poll::Ready(_) => {
                    this.read_sleep.set(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        } else {
            this.read_sleep.set(Some(Box::pin(sleep(*this.delay))));
        }

        Pin::new(&mut **this.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for LatencyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut this = self.project();

        if let Some(sleep_fut) = this.write_sleep.as_mut().as_pin_mut() {
            match sleep_fut.poll(cx) {
                Poll::Ready(_) => {
                    this.write_sleep.set(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        } else {
            this.write_sleep.set(Some(Box::pin(sleep(*this.delay))));
        }

        Pin::new(&mut **this.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut this = self.project();
        Pin::new(&mut **this.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut this = self.project();
        Pin::new(&mut **this.stream).poll_shutdown(cx)
    }
}
