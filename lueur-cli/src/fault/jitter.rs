// src/fault/jitter.rs

use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use async_trait::async_trait;
use axum::http;
use pin_project::pin_project;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;
use tokio::time::Duration;
use tokio::time::Sleep;
use tokio::time::sleep;

use super::Bidirectional;
use super::FaultInjector;
use crate::event::ProxyTaskEvent;
use crate::types::Direction;

/// Enumeration of jitter strategies.
#[derive(Debug, Clone)]
pub enum JitterStrategy {
    Default { amplitude: Duration, frequency: f64 },
}

/// Options for jitter injection.
#[derive(Debug, Clone)]
pub struct JitterOptions {
    pub strategy: JitterStrategy,
}

/// Jitter Injector that introduces variable delays based on amplitude and
/// frequency.
#[derive(Debug)]
pub struct JitterInjector {
    options: JitterOptions,
}

impl JitterInjector {
    pub fn new(options: JitterOptions) -> Self {
        Self { options }
    }

    /// Determines whether to inject jitter based on the configured frequency.
    fn should_jitter(&self, rng: &mut SmallRng) -> bool {
        match &self.options.strategy {
            JitterStrategy::Default { frequency, .. } => {
                rng.r#gen::<f64>() < *frequency
            } // Implement other strategies here.
        }
    }

    /// Generates a random jitter duration based on the configured amplitude.
    fn generate_jitter(&self, rng: &mut SmallRng) -> Duration {
        match &self.options.strategy {
            JitterStrategy::Default { amplitude, .. } => {
                // Generate a random duration between 0 and amplitude.
                let millis = rng.gen_range(0..=amplitude.as_millis() as u64);
                Duration::from_millis(millis)
            } // Implement other strategies here.
        }
    }
}

impl Clone for JitterInjector {
    fn clone(&self) -> Self {
        Self { options: self.options.clone() }
    }
}

#[async_trait]
impl FaultInjector for JitterInjector {
    /// Injects jitter into a bidirectional stream.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        direction: &Direction,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static> {
        Box::new(JitterStream::new(stream, self.clone(), direction))
    }

    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, crate::errors::ProxyError> {
        Ok(resp)
    }

    async fn apply_on_request_builder(
        &self,
        builder: reqwest::ClientBuilder,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, crate::errors::ProxyError> {
        Ok(builder)
    }

    async fn apply_on_request(
        &self,
        request: reqwest::Request,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::Request, crate::errors::ProxyError> {
        Ok(request)
    }
}

/// A wrapper around a bidirectional stream that injects jitter.
#[pin_project]
pub struct JitterStream {
    #[pin]
    stream: Box<dyn Bidirectional + 'static>,
    #[pin]
    injector: JitterInjector,
    #[pin]
    rng: SmallRng,
    #[pin]
    read_sleep: Option<Pin<Box<Sleep>>>,
    #[pin]
    write_sleep: Option<Pin<Box<Sleep>>>,
    #[pin]
    direction: Direction,
}

impl JitterStream {
    /// Creates a new JitterStream.
    fn new(
        stream: Box<dyn Bidirectional + 'static>,
        injector: JitterInjector,
        direction: &Direction,
    ) -> Self {
        Self {
            stream,
            injector,
            rng: SmallRng::from_entropy(),
            read_sleep: None,
            write_sleep: None,
            direction: direction.clone(),
        }
    }
}

impl AsyncRead for JitterStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut this = self.project();

        if this.direction.is_ingress() {
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
                let injector = this.injector;
                let mut rng = this.rng;
                if injector.should_jitter(&mut rng) {
                    let jitter_duration = injector.generate_jitter(&mut rng);
                    this.read_sleep.set(Some(Box::pin(sleep(jitter_duration))));
                }
            }
        }

        Pin::new(&mut **this.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for JitterStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut this = self.project();

        if this.direction.is_egress() {
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
                let injector = this.injector;
                let mut rng = this.rng;
                if injector.should_jitter(&mut rng) {
                    let jitter_duration = injector.generate_jitter(&mut rng);
                    this.write_sleep
                        .set(Some(Box::pin(sleep(jitter_duration))));
                }
            }
        }

        Pin::new(&mut **this.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
