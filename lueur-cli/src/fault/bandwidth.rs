use std::io::Cursor;
use std::io::Result as IoResult;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use async_trait::async_trait;
use axum::http;
use bytes::BytesMut;
use futures::StreamExt;
use governor::Quota;
use governor::RateLimiter;
use governor::clock::DefaultClock;
use governor::state::InMemoryState;
use governor::state::direct::NotKeyed;
use hyper::http::Response;
use pin_project::pin_project;
use reqwest::Body;
use reqwest::ClientBuilder as ReqwestClientBuilder;
use reqwest::Request as ReqwestRequest;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;
use tokio::io::split;
use tokio_util::io::ReaderStream;

use super::Bidirectional;
use super::FaultInjector;
use crate::errors::ProxyError;
use crate::event::FaultEvent;
use crate::event::ProxyTaskEvent;
use crate::types::BandwidthUnit;
use crate::types::Direction;

/// Enumeration of bandwidth throttling strategies.
#[derive(Debug, Clone)]
pub enum BandwidthStrategy {
    Single {
        rate: usize,         // Bandwidth rate as specified by the user
        unit: BandwidthUnit, // Unit for the bandwidth rate
    },
    Dual {
        ingress_rate: usize,
        ingress_unit: BandwidthUnit,
        egress_rate: usize,
        egress_unit: BandwidthUnit,
    },
}

/// Options for jitter injection.
#[derive(Debug, Clone)]
pub struct BandwidthOptions {
    pub strategy: BandwidthStrategy,
}

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// BandwidthLimitedWrite wraps an AsyncWrite stream and limits the write
/// bandwidth.
#[pin_project]
pub struct BandwidthLimitedWrite<S> {
    #[pin]
    inner: S,
    limiter: Option<Arc<Limiter>>,
    #[pin]
    delay: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    event: Option<Box<dyn ProxyTaskEvent>>,
    max_write_size: usize,
}

impl<S> BandwidthLimitedWrite<S>
where
    S: AsyncWrite + Unpin,
{
    /// Creates a new BandwidthLimitedWrite with the specified bandwidth
    /// options.
    ///
    /// # Arguments
    ///
    /// * `inner` - The underlying stream to wrap.
    /// * `options` - The bandwidth throttling options.
    /// * `event` - An optional event handler for fault events.
    pub fn new(
        inner: S,
        options: BandwidthOptions,
        event: Option<Box<dyn ProxyTaskEvent>>,
    ) -> Result<Self, S> {
        // Initialize egress limiter based on strategy
        let (limiter, max_write_size) = match options.strategy {
            BandwidthStrategy::Single { rate, unit } => {
                let rate_bps = unit.to_bytes_per_second(rate);
                if rate_bps == 0 {
                    return Err(inner);
                }
                // Attempt to create a NonZeroU32 quota
                let quota = match NonZeroU32::new(rate_bps as u32) {
                    Some(q) => Quota::per_second(q),
                    None => return Err(inner), /* Fail if quota cannot be
                                                * created */
                };
                (Some(Arc::new(RateLimiter::direct(quota))), rate_bps)
            }
            BandwidthStrategy::Dual {
                ingress_rate: _,
                ingress_unit: _,
                egress_rate,
                egress_unit,
            } => {
                let egress_bps = egress_unit.to_bytes_per_second(egress_rate);
                if egress_bps == 0 {
                    return Err(inner);
                }
                // Attempt to create a NonZeroU32 quota
                let quota = match NonZeroU32::new(egress_bps as u32) {
                    Some(q) => Quota::per_second(q),
                    None => return Err(inner), /* Fail if quota cannot be
                                                * created */
                };
                (Some(Arc::new(RateLimiter::direct(quota))), egress_bps)
            }
        };

        Ok(BandwidthLimitedWrite {
            inner,
            limiter,
            delay: None,
            event,
            max_write_size,
        })
    }
}

/// BandwidthLimitedRead wraps an AsyncRead stream and limits the read
/// bandwidth.
#[pin_project]
pub struct BandwidthLimitedRead<S> {
    #[pin]
    inner: S,
    limiter: Option<Arc<Limiter>>,
    #[pin]
    delay: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    event: Option<Box<dyn ProxyTaskEvent>>,
    max_read_size: usize,
}

impl<S> BandwidthLimitedRead<S>
where
    S: AsyncRead + Unpin,
{
    /// Creates a new BandwidthLimitedRead with the specified bandwidth options.
    ///
    /// # Arguments
    ///
    /// * `inner` - The underlying stream to wrap.
    /// * `options` - The bandwidth throttling options.
    /// * `event` - An optional event handler for fault events.
    pub fn new(
        inner: S,
        options: BandwidthOptions,
        event: Option<Box<dyn ProxyTaskEvent>>,
    ) -> Result<Self, S> {
        // Initialize ingress limiter based on strategy
        let (limiter, max_read_size) = match options.strategy {
            BandwidthStrategy::Single { rate, unit } => {
                let rate_bps = unit.to_bytes_per_second(rate);
                if rate_bps == 0 {
                    return Err(inner);
                }
                let quota = match NonZeroU32::new(rate_bps as u32) {
                    Some(q) => Quota::per_second(q),
                    None => return Err(inner), /* Fail if quota cannot be
                                                * created */
                };
                (Some(Arc::new(RateLimiter::direct(quota))), rate_bps)
            }
            BandwidthStrategy::Dual {
                ingress_rate,
                ingress_unit,
                egress_rate: _,
                egress_unit: _,
            } => {
                let ingress_bps =
                    ingress_unit.to_bytes_per_second(ingress_rate);
                if ingress_bps == 0 {
                    return Err(inner);
                }
                // Attempt to create a NonZeroU32 quota
                let quota = match NonZeroU32::new(ingress_bps as u32) {
                    Some(q) => Quota::per_second(q),
                    None => return Err(inner), /* Fail if quota cannot be
                                                * created */
                };
                (Some(Arc::new(RateLimiter::direct(quota))), ingress_bps)
            }
        };

        Ok(BandwidthLimitedRead {
            inner,
            limiter,
            delay: None,
            event,
            max_read_size,
        })
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BandwidthLimitedWrite<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        let mut this = self.project();

        if let Some(limiter) = this.limiter.as_ref().cloned() {
            let requested_write = buf.len();
            let to_write = std::cmp::min(*this.max_write_size, requested_write);
            if to_write == 0 {
                return Poll::Ready(Ok(0));
            }

            let permit_nz = match NonZeroU32::new(to_write as u32) {
                Some(nz) => nz,
                None => {
                    // Handle the case where to_write exceeds u32::MAX
                    return Poll::Ready(Ok(0));
                }
            };

            match limiter.check_n(permit_nz) {
                Ok(_) => {
                    match Pin::new(&mut this.inner).poll_write(cx, buf) {
                        Poll::Ready(Ok(written)) => {
                            // Emit event
                            if let Some(event) = &*this.event {
                                let _ = event.on_applied(
                                    FaultEvent::Bandwidth {
                                        bps: Some(written),
                                    },
                                    Direction::Egress,
                                );
                            }
                            Poll::Ready(Ok(written))
                        }
                        Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                        Poll::Pending => Poll::Pending,
                    }
                }
                Err(_) => {
                    // Rate limit exceeded
                    if this.delay.is_none() {
                        let limiter_clone = limiter.clone();
                        let delay_future = async move {
                            limiter_clone.until_ready().await;
                        };
                        *this.delay = Some(Box::pin(delay_future));
                    }

                    if let Some(ref mut delay) = *this.delay {
                        match delay.as_mut().poll(cx) {
                            Poll::Ready(_) => {
                                // Delay completed, reset the delay
                                *this.delay = None;
                                // Return Poll::Pending to allow re-polling
                                return Poll::Pending;
                            }
                            Poll::Pending => {
                                // Still waiting
                                return Poll::Pending;
                            }
                        }
                    }

                    // No delay set, return Poll::Pending
                    Poll::Pending
                }
            }
        } else {
            // No limiter, proceed normally
            Pin::new(&mut this.inner).poll_write(cx, buf)
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        let mut this = self.project();
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        let mut this = self.project();
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for BandwidthLimitedRead<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let mut this = self.project();

        if let Some(limiter) = this.limiter.as_ref().cloned() {
            // Cap the read size to the maximum allowed
            let requested_read = buf.remaining();
            let to_read = std::cmp::min(*this.max_read_size, requested_read);
            if to_read == 0 {
                return Poll::Ready(Ok(()));
            }

            // Attempt to reserve `to_read` permits
            let permit_nz = match NonZeroU32::new(to_read as u32) {
                Some(nz) => nz,
                None => {
                    // Handle the case where to_read exceeds u32::MAX
                    return Poll::Ready(Ok(()));
                }
            };

            match limiter.check_n(permit_nz) {
                Ok(_) => {
                    // Permits are available; proceed to read
                    let mut limited_buf =
                        ReadBuf::new(&mut buf.initialize_unfilled()[..to_read]);

                    match this.inner.poll_read(cx, &mut limited_buf) {
                        Poll::Ready(Ok(())) => {
                            let filled = limited_buf.filled().len();
                            buf.advance(filled);

                            if let Some(event) = &*this.event {
                                let _ = event.on_applied(
                                    FaultEvent::Bandwidth { bps: Some(filled) },
                                    Direction::Ingress,
                                );
                            }

                            Poll::Ready(Ok(()))
                        }
                        Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                        Poll::Pending => Poll::Pending,
                    }
                }
                Err(_) => {
                    // Rate limit exceeded
                    if this.delay.is_none() {
                        let limiter_clone = limiter.clone();
                        let delay_future = async move {
                            limiter_clone.until_ready().await;
                        };
                        *this.delay = Some(Box::pin(delay_future));
                    }

                    if let Some(ref mut delay) = *this.delay {
                        match delay.as_mut().poll(cx) {
                            Poll::Ready(_) => {
                                // Delay completed, reset the delay
                                *this.delay = None;
                                // Return Poll::Pending to allow re-polling
                                Poll::Pending
                            }
                            Poll::Pending => {
                                // Still waiting
                                Poll::Pending
                            }
                        }
                    } else {
                        // No delay set, return Poll::Pending
                        Poll::Pending
                    }
                }
            }
        } else {
            // No limiter, proceed normally
            this.inner.poll_read(cx, buf)
        }
    }
}

/// A bidirectional stream that wraps limited reader and writer.
#[pin_project]
struct BandwidthLimitedBidirectional<R, W> {
    #[pin]
    reader: R,
    #[pin]
    writer: W,
}

impl<R, W> BandwidthLimitedBidirectional<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> AsyncRead for BandwidthLimitedBidirectional<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        self.project().reader.poll_read(cx, buf)
    }
}

impl<R, W> AsyncWrite for BandwidthLimitedBidirectional<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        self.project().writer.poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        self.project().writer.poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        self.project().writer.poll_shutdown(cx)
    }
}

#[derive(Debug, Clone)]
pub struct BandwidthLimitFaultInjector {
    pub options: BandwidthOptions,
}

impl BandwidthLimitFaultInjector {
    pub fn new(options: BandwidthOptions) -> BandwidthLimitFaultInjector {
        BandwidthLimitFaultInjector { options }
    }
}

// TokioAsyncRead + TokioAsyncWrite + Unpin + Send

#[async_trait]
impl FaultInjector for BandwidthLimitFaultInjector {
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        direction: &Direction,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static> {
        let (read_half, write_half) = split(stream);

        let _ = event
            .with_fault(FaultEvent::Bandwidth { bps: None }, direction.clone());

        // Wrap the read half if ingress or both directions are specified
        let limited_read: Box<dyn AsyncRead + Unpin + Send> =
            if direction.is_ingress() {
                match BandwidthLimitedRead::new(
                    read_half,
                    self.options.clone(),
                    Some(event.clone()),
                ) {
                    Ok(lr) => Box::new(lr),
                    Err(rh) => {
                        Box::new(rh) // Fallback to the original read half
                    }
                }
            } else {
                Box::new(read_half) as Box<dyn AsyncRead + Unpin + Send>
            };

        // Wrap the write half if egress or both directions are specified
        let limited_write: Box<dyn AsyncWrite + Send + Unpin> =
            if direction.is_egress() {
                match BandwidthLimitedWrite::new(
                    write_half,
                    self.options.clone(),
                    Some(event.clone()),
                ) {
                    Ok(lw) => Box::new(lw),
                    Err(wh) => Box::new(wh),
                }
            } else {
                Box::new(write_half) as Box<dyn AsyncWrite + Unpin + Send>
            };

        // Combine the limited read and write into a new bidirectional stream
        let limited_bidirectional =
            BandwidthLimitedBidirectional::new(limited_read, limited_write);

        Box::new(limited_bidirectional)
    }

    async fn apply_on_request_builder(
        &self,
        builder: ReqwestClientBuilder,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestClientBuilder, ProxyError> {
        Ok(builder)
    }

    /// Applies bandwidth limiting to an outgoing request.
    async fn apply_on_request(
        &self,
        request: ReqwestRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError> {
        let _ = event
            .with_fault(FaultEvent::Bandwidth { bps: None }, Direction::Egress);

        let original_body = request.body();
        if let Some(body) = original_body {
            if let Some(bytes) = body.as_bytes() {
                let owned_bytes = Cursor::new(bytes.to_vec());

                // Wrap the owned bytes with BandwidthLimitedRead
                let bandwidth_limited_read = BandwidthLimitedRead::new(
                    owned_bytes,
                    self.options.clone(),
                    Some(event.clone()),
                )
                .unwrap();

                let reader_stream = ReaderStream::new(bandwidth_limited_read);

                let new_body = Body::wrap_stream(reader_stream);
                let mut builder = request.try_clone().ok_or_else(|| {
                    ProxyError::Other("Couldn't clone request".into())
                })?;
                *builder.body_mut() = Some(new_body);

                Ok(builder)
            } else {
                // If the body doesn't have bytes, leave it unchanged
                Ok(request)
            }
        } else {
            // If there's no body, leave the request unchanged
            Ok(request)
        }
    }

    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError> {
        let _ = event.with_fault(
            FaultEvent::Bandwidth { bps: None },
            Direction::Ingress,
        );

        let (parts, body) = resp.into_parts();
        let version = parts.version;
        let status = parts.status;
        let headers = parts.headers.clone();

        let owned_body = Cursor::new(body);

        let reader = BandwidthLimitedRead::new(
            owned_body,
            self.options.clone(),
            Some(event.clone()),
        )
        .unwrap();

        let mut reader_stream = ReaderStream::new(reader);

        let mut buffer = BytesMut::new();
        while let Some(chunk) = reader_stream.next().await {
            buffer.extend_from_slice(&chunk?);
        }
        let response_body = buffer.to_vec();

        // Reconstruct the HTTP response with the limited body
        let mut intermediate = Response::new(response_body);
        *intermediate.version_mut() = version;
        *intermediate.status_mut() = status;
        *intermediate.headers_mut() = headers;

        Ok(intermediate)
    }
}
