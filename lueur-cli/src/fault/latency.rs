use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use axum::async_trait;
use axum::http;
use pin_project::pin_project;
use rand::SeedableRng;
use rand::rngs::SmallRng;
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
use crate::config::LatencySettings;
use crate::event::FaultEvent;
use crate::event::ProxyTaskEvent;
use crate::types::Direction;
use crate::types::LatencyDistribution;

#[derive(Debug)]
pub struct LatencyInjector {
    settings: LatencySettings,
}

impl From<&LatencySettings> for LatencyInjector {
    fn from(settings: &LatencySettings) -> Self {
        LatencyInjector { settings: settings.clone() }
    }
}

impl Clone for LatencyInjector {
    fn clone(&self) -> Self {
        Self { settings: self.settings.clone() }
    }
}

impl LatencyInjector {
    fn get_delay(&self, rng: &mut SmallRng) -> Duration {
        match &self.settings.distribution {
            LatencyDistribution::Normal => {
                let normal = Normal::new(
                    self.settings.latency_mean,
                    self.settings.latency_stddev,
                )
                .unwrap();
                let mut sample = normal.sample(rng);
                while sample < 0.0 {
                    sample = normal.sample(rng);
                }

                let millis = sample.floor() as u64;
                let nanos =
                    ((sample - millis as f64) * 1_000_000.0).round() as u32;
                Duration::from_millis(millis)
                    + Duration::from_nanos(nanos as u64)
            }
            LatencyDistribution::Pareto => {
                let pareto = Pareto::new(
                    self.settings.latency_scale,
                    self.settings.latency_shape,
                )
                .unwrap();
                let mut sample = pareto.sample(rng);
                while sample < 0.0 {
                    sample = pareto.sample(rng);
                }

                let millis = sample.floor() as u64;
                let nanos =
                    ((sample - millis as f64) * 1_000_000.0).round() as u32;
                Duration::from_millis(millis)
                    + Duration::from_nanos(nanos as u64)
            }
            LatencyDistribution::ParetoNormal => {
                let pareto = Pareto::new(
                    self.settings.latency_scale,
                    self.settings.latency_shape,
                )
                .unwrap();
                let mut pareto_sample = pareto.sample(rng);
                while pareto_sample < 0.0 {
                    pareto_sample = pareto.sample(rng);
                }

                let normal = Normal::new(
                    self.settings.latency_mean,
                    self.settings.latency_stddev,
                )
                .unwrap();
                let mut normal_sample = normal.sample(rng);
                while normal_sample < 0.0 {
                    normal_sample = normal.sample(rng);
                }

                let total = pareto_sample + normal_sample;
                let millis = total.floor() as u64;
                let nanos =
                    ((total - millis as f64) * 1_000_000.0).round() as u32;
                Duration::from_millis(millis)
                    + Duration::from_nanos(nanos as u64)
            }
            LatencyDistribution::Uniform => {
                let uniform = Uniform::new(
                    self.settings.latency_min,
                    self.settings.latency_max,
                );
                let mut sample = uniform.sample(rng);
                while sample < 0.0 {
                    sample = uniform.sample(rng);
                }

                let millis = sample.floor() as u64;
                let nanos =
                    ((sample - millis as f64) * 1_000_000.0).round() as u32;
                Duration::from_millis(millis)
                    + Duration::from_nanos(nanos as u64)
            }
        }
    }
}

#[async_trait]
impl FaultInjector for LatencyInjector {
    /// Injects latency into a bidirectional stream.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static> {
        let direction = self.settings.direction.clone();
        let _ = event
            .with_fault(FaultEvent::Latency { delay: None }, direction.clone());
        Box::new(LatencyStream::new(
            stream,
            self.clone(),
            &direction,
            Some(event),
        ))
    }

    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, crate::errors::ProxyError> {
        let mut rng = SmallRng::from_entropy();
        let delay = self.get_delay(&mut rng);
        tracing::debug!("Adding latency {:?}", delay);
        let _ = event.with_fault(
            FaultEvent::Latency { delay: None },
            Direction::Ingress,
        );
        let _ = event.on_applied(
            FaultEvent::Latency { delay: Some(delay) },
            Direction::Ingress,
        );
        sleep(delay).await;
        Ok(resp)
    }

    async fn apply_on_request_builder(
        &self,
        builder: reqwest::ClientBuilder,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, crate::errors::ProxyError> {
        Ok(builder)
    }

    async fn apply_on_request(
        &self,
        request: reqwest::Request,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::Request, crate::errors::ProxyError> {
        Ok(request)
    }
}

#[pin_project]
pub struct LatencyStream {
    #[pin]
    stream: Box<dyn Bidirectional + 'static>,
    #[pin]
    injector: LatencyInjector,
    #[pin]
    rng: SmallRng,
    direction: Direction,
    event: Option<Box<dyn ProxyTaskEvent>>,
    read_sleep: Option<Pin<Box<Sleep>>>,
    write_sleep: Option<Pin<Box<Sleep>>>,
}

impl LatencyStream {
    fn new(
        stream: Box<dyn Bidirectional + 'static>,
        injector: LatencyInjector,
        direction: &Direction,
        event: Option<Box<dyn ProxyTaskEvent>>,
    ) -> Self {
        Self {
            stream,
            injector,
            event,
            rng: SmallRng::from_entropy(),
            read_sleep: None,
            write_sleep: None,
            direction: direction.clone(),
        }
    }
}

impl AsyncRead for LatencyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut this = self.project();

        if this.direction.is_ingress() {
            let injector = this.injector;
            let mut rng = this.rng;
            let delay = injector.get_delay(&mut rng);
            if this.read_sleep.is_none() {
                let event = this.event;
                if event.is_some() {
                    let _ = event.clone().unwrap().on_applied(
                        FaultEvent::Latency { delay: Some(delay) },
                        Direction::Ingress,
                    );
                }
                this.read_sleep.replace(Box::pin(sleep(delay)));
            }

            if let Some(delay) = this.read_sleep.as_mut() {
                match delay.as_mut().poll(cx) {
                    Poll::Ready(_) => {
                        this.read_sleep.take();
                        return this.stream.poll_read(cx, buf);
                    }
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                }
            }
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

        if this.direction.is_egress() {
            let injector = this.injector;
            let mut rng = this.rng;
            let delay = injector.get_delay(&mut rng);

            if this.write_sleep.is_none() {
                let event = this.event;
                if event.is_some() {
                    let _ = event.clone().unwrap().on_applied(
                        FaultEvent::Latency { delay: Some(delay) },
                        Direction::Egress,
                    );
                }
                this.write_sleep.replace(Box::pin(sleep(delay)));
            }

            if let Some(delay) = this.write_sleep.as_mut() {
                match delay.as_mut().poll(cx) {
                    Poll::Ready(_) => {
                        this.write_sleep.take();
                        return this.stream.poll_write(cx, buf);
                    }
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                }
            }
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
