use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use futures::TryStreamExt;
use hyper::http::Response;
use reqwest::Body;
use reqwest::ClientBuilder as ReqwestClientBuilder;
use reqwest::Request as ReqwestRequest;
use reqwest::Response as ReqwestResponse;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::ReadBuf;
use tokio::time::sleep;
use tokio_util::io::ReaderStream;
use tokio_util::io::StreamReader;

use super::Bidirectional;
use super::FaultInjector;
use crate::errors::ProxyError;

/// Enumeration of jitter strategies.
#[derive(Debug, Clone)]
pub enum BandwidthStrategy {
    Default { bps: usize },
}

/// Options for jitter injection.
#[derive(Debug, Clone)]
pub struct BandwidthOptions {
    pub strategy: BandwidthStrategy,
}

struct BandwidthLimitedStream<S> {
    inner: S,
    bytes_per_second: usize,
    tokens: usize,
    last_refill: Instant,
}

impl<S: AsyncRead + Unpin> AsyncRead for BandwidthLimitedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.get_mut();

        this.maybe_refill_tokens();

        let inner = Pin::new(&mut this.inner);

        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        if this.tokens == 0 {
            let wait_time = this.time_until_refill();
            tracing::debug!("Waiting for {:?}", wait_time);
            if let Some(dur) = wait_time {
                let waker = cx.waker().clone();
                let fut = Box::pin(async move {
                    sleep(dur).await;
                    waker.wake_by_ref();
                });
                tokio::spawn(fut);
                return Poll::Pending;
            } else {
                return Poll::Pending;
            }
        }

        let allowed = this.tokens.min(buf.remaining());
        let mut temp_buf = vec![0u8; allowed];
        let mut temp_read_buf = ReadBuf::new(&mut temp_buf);

        match inner.poll_read(cx, &mut temp_read_buf)? {
            Poll::Ready(()) => {
                let filled = temp_read_buf.filled().len();
                if filled > 0 {
                    buf.put_slice(&temp_buf[..filled]);
                    this.tokens = this.tokens.saturating_sub(filled);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BandwidthLimitedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        let this = self.get_mut();

        this.maybe_refill_tokens();

        let inner = Pin::new(&mut this.inner);

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        if this.tokens == 0 {
            let wait_time = this.time_until_refill();
            if let Some(dur) = wait_time {
                let waker = cx.waker().clone();
                let fut = Box::pin(async move {
                    sleep(dur).await;
                    waker.wake_by_ref();
                });
                tokio::spawn(fut);
                return Poll::Pending;
            } else {
                return Poll::Pending;
            }
        }

        let allowed = this.tokens.min(buf.len());
        let to_write = &buf[..allowed];
        match inner.poll_write(cx, to_write)? {
            Poll::Ready(written) => {
                this.tokens = this.tokens.saturating_sub(written);
                Poll::Ready(Ok(written))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S> BandwidthLimitedStream<S> {
    fn new(inner: S, bytes_per_second: usize) -> Self {
        Self {
            inner,
            bytes_per_second,
            tokens: bytes_per_second,
            last_refill: Instant::now(),
        }
    }

    fn maybe_refill_tokens(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_refill) >= Duration::from_secs(1) {
            self.tokens = self.bytes_per_second;
            self.last_refill = now;
        }
    }

    fn time_until_refill(&self) -> Option<Duration> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        if elapsed < Duration::from_secs(1) {
            Some(Duration::from_secs(1) - elapsed)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct BandwidthLimitFaultInjector {
    options: BandwidthOptions,
}

impl BandwidthLimitFaultInjector {
    pub fn new(options: BandwidthOptions) -> Self {
        Self { options }
    }

    fn get_bps(&self) -> usize {
        match self.options.strategy {
            BandwidthStrategy::Default { bps } => bps,
        }
    }
}

#[async_trait]
impl FaultInjector for BandwidthLimitFaultInjector {
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
    ) -> Box<dyn Bidirectional + 'static> {
        let wrapped = BandwidthLimitedStream::new(stream, self.get_bps());
        Box::new(wrapped)
    }

    async fn apply_on_request_builder(
        &self,
        builder: ReqwestClientBuilder,
    ) -> Result<ReqwestClientBuilder, ProxyError> {
        Ok(builder)
    }

    async fn apply_on_request(
        &self,
        request: ReqwestRequest,
    ) -> Result<ReqwestRequest, ProxyError> {
        let original_body = request.body();
        if let Some(body) = original_body {
            if let Some(bytes) = body.as_bytes() {
                let (mut w, r) = tokio::io::duplex(bytes.len());
                w.write_all(bytes).await.map_err(ProxyError::IoError)?;
                drop(w);
                let rl_stream = BandwidthLimitedStream::new(r, self.get_bps());
                let reader_stream = ReaderStream::new(rl_stream);
                let new_body = Body::wrap_stream(reader_stream);
                let mut builder = request.try_clone().ok_or_else(|| {
                    ProxyError::Other("Couldn't clone request".into())
                })?;
                *builder.body_mut() = Some(new_body);
                Ok(builder)
            } else {
                Ok(request)
            }
        } else {
            Ok(request)
        }
    }

    async fn apply_on_response(
        &self,
        resp: ReqwestResponse,
    ) -> Result<ReqwestResponse, ProxyError> {
        let version = resp.version();
        let status = resp.status();
        let headers = resp.headers().clone();

        let stream =
            resp.bytes_stream().map_err(|e| IoError::new(ErrorKind::Other, e));
        let async_read = StreamReader::new(stream);
        let limited = BandwidthLimitedStream::new(async_read, self.get_bps());
        let reader_stream = ReaderStream::new(limited);
        let new_body = Body::wrap_stream(reader_stream);

        // we need to go through a hyper::http::Response first before reworking
        // it into a reqwest::Response
        let mut intermediate = Response::new(new_body);
        *intermediate.version_mut() = version;
        *intermediate.status_mut() = status;
        *intermediate.headers_mut() = headers;

        let new_resp = ReqwestResponse::from(intermediate);

        Ok(new_resp)
    }
}
