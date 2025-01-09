use std::io::Cursor;
use std::io::Result as IoResult;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use async_trait::async_trait;
use axum::http;
use bytes::BytesMut;
use futures::StreamExt;
use pin_project::pin_project;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use reqwest::Body;
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
use crate::types::Direction;

/// Enumeration of packet loss strategies.
#[derive(Debug, Clone)]
pub enum PacketLossStrategy {
    /// Multi-State Markov Model packet loss strategy.
    MultiStateMarkov {
        transition_matrix: Vec<Vec<f64>>, /* Rows and columns correspond to
                                           * states */
        loss_probabilities: Vec<f64>, // Packet loss probability for each state
    },
}

impl Default for PacketLossStrategy {
    fn default() -> Self {
        PacketLossStrategy::MultiStateMarkov {
            transition_matrix: vec![
                // Excellent state transitions
                vec![0.9, 0.1, 0.0, 0.0, 0.0],
                // Good state transitions
                vec![0.05, 0.9, 0.05, 0.0, 0.0],
                // Fair state transitions
                vec![0.0, 0.1, 0.8, 0.1, 0.0],
                // Poor state transitions
                vec![0.0, 0.0, 0.2, 0.7, 0.1],
                // Bad state transitions
                vec![0.0, 0.0, 0.0, 0.3, 0.7],
            ],
            loss_probabilities: vec![
                0.0,  // Excellent
                0.01, // Good
                0.05, // Fair
                0.1,  // Poor
                0.2,  // Bad
            ],
        }
    }
}

/// Options for packet loss injection.
#[derive(Debug, Clone)]
pub struct PacketLossOptions {
    pub strategy: PacketLossStrategy,
}

/// Enumeration representing the different network states in the Markov model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiState {
    Excellent,
    Good,
    Fair,
    Poor,
    Bad,
    // Extend with more states if needed
}

impl MultiState {
    /// Creates a `MultiState` from a given index.
    fn from_index(index: usize) -> Self {
        match index {
            0 => MultiState::Excellent,
            1 => MultiState::Good,
            2 => MultiState::Fair,
            3 => MultiState::Poor,
            4 => MultiState::Bad,
            _ => MultiState::Good, // Default fallback
        }
    }

    /// Converts a `MultiState` to its corresponding index.
    fn to_index(&self) -> usize {
        match self {
            MultiState::Excellent => 0,
            MultiState::Good => 1,
            MultiState::Fair => 2,
            MultiState::Poor => 3,
            MultiState::Bad => 4,
        }
    }
}

/// PacketLossLimitedRead wraps an AsyncRead stream and applies packet loss
/// based on the Multi-State Markov Model.
#[pin_project]
pub struct PacketLossLimitedRead<S> {
    #[pin]
    inner: S,
    strategy: PacketLossStrategy,
    state: MultiState,                        // Current state
    transition_matrix: Option<Vec<Vec<f64>>>, // Only for MultiStateMarkov
    loss_probabilities: Option<Vec<f64>>,     // Only for MultiStateMarkov
    event: Option<Box<dyn ProxyTaskEvent>>,
    rng: SmallRng,
}

impl<S> PacketLossLimitedRead<S> {
    /// Creates a new PacketLossLimitedRead.
    ///
    /// # Arguments
    ///
    /// * `inner` - The underlying AsyncRead stream.
    /// * `options` - The packet loss options.
    /// * `event` - An optional event handler for fault events.
    pub fn new(
        inner: S,
        options: PacketLossOptions,
        event: Option<Box<dyn ProxyTaskEvent>>,
    ) -> Self {
        let (transition_matrix, loss_probabilities) = match options.strategy {
            PacketLossStrategy::MultiStateMarkov {
                ref transition_matrix,
                ref loss_probabilities,
            } => (
                Some(transition_matrix.clone()),
                Some(loss_probabilities.clone()),
            ),
        };

        let initial_state = MultiState::Good; // Starting state for Markov model

        PacketLossLimitedRead {
            inner,
            strategy: options.strategy,
            state: initial_state,
            transition_matrix,
            loss_probabilities,
            event,
            rng: SmallRng::from_entropy(),
        }
    }
}

impl<S> AsyncRead for PacketLossLimitedRead<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let mut this = self.project();
        // Determine if the packet should be dropped based on the current state
        let should_drop = match &this.strategy {
            PacketLossStrategy::MultiStateMarkov {
                transition_matrix,
                loss_probabilities,
            } => {
                // Transition to the next state based on the current state
                let current_index = this.state.to_index();
                let transition_probs = &transition_matrix[current_index];
                let rand_val = this.rng.r#gen::<f64>();
                let mut cumulative = 0.0;
                let mut new_state = current_index;
                for (i, &prob) in transition_probs.iter().enumerate() {
                    cumulative += prob;
                    if rand_val < cumulative {
                        new_state = i;
                        break;
                    }
                }
                *this.state = MultiState::from_index(new_state);

                // Determine packet loss based on the new state's loss
                // probability
                this.rng.r#gen::<f64>() < loss_probabilities[new_state]
            }
        };

        if should_drop {
            // Drop the packet by not reading anything and returning Ok with 0
            // bytes
            if let Some(event) = &*this.event {
                let _ = event
                    .on_applied(FaultEvent::PacketLoss {}, Direction::Ingress);
            }
            buf.advance(0);
            Poll::Ready(Ok(()))
        } else {
            // Proceed to read normally
            this.inner.poll_read(cx, buf)
        }
    }
}

/// PacketLossLimitedWrite wraps an AsyncWrite stream and applies packet loss
/// based on the Multi-State Markov Model.
#[pin_project]
pub struct PacketLossLimitedWrite<S> {
    #[pin]
    inner: S,
    strategy: PacketLossStrategy,
    state: MultiState,                        // Current state
    transition_matrix: Option<Vec<Vec<f64>>>, // Only for MultiStateMarkov
    loss_probabilities: Option<Vec<f64>>,     // Only for MultiStateMarkov
    event: Option<Box<dyn ProxyTaskEvent>>,
    rng: SmallRng,
}

impl<S> PacketLossLimitedWrite<S> {
    /// Creates a new PacketLossLimitedWrite.
    ///
    /// # Arguments
    ///
    /// * `inner` - The underlying AsyncWrite stream.
    /// * `options` - The packet loss options.
    /// * `event` - An optional event handler for fault events.
    pub fn new(
        inner: S,
        options: PacketLossOptions,
        event: Option<Box<dyn ProxyTaskEvent>>,
    ) -> Self {
        let (transition_matrix, loss_probabilities) = match options.strategy {
            PacketLossStrategy::MultiStateMarkov {
                ref transition_matrix,
                ref loss_probabilities,
            } => (
                Some(transition_matrix.clone()),
                Some(loss_probabilities.clone()),
            ),
        };

        let initial_state = MultiState::Good; // Starting state for Markov model

        PacketLossLimitedWrite {
            inner,
            strategy: options.strategy,
            state: initial_state,
            transition_matrix,
            loss_probabilities,
            event,
            rng: SmallRng::from_entropy(),
        }
    }
}

impl<S> AsyncWrite for PacketLossLimitedWrite<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        let mut this = self.project();
        // Determine if the packet should be dropped based on the current state
        let should_drop = match &this.strategy {
            PacketLossStrategy::MultiStateMarkov {
                transition_matrix,
                loss_probabilities,
            } => {
                // Transition to the next state based on the current state
                let current_index = this.state.to_index();
                let transition_probs = &transition_matrix[current_index];
                let rand_val = this.rng.r#gen::<f64>();
                let mut cumulative = 0.0;
                let mut new_state = current_index;
                for (i, &prob) in transition_probs.iter().enumerate() {
                    cumulative += prob;
                    if rand_val < cumulative {
                        new_state = i;
                        break;
                    }
                }
                *this.state = MultiState::from_index(new_state);

                // Determine packet loss based on the new state's loss
                // probability
                this.rng.r#gen::<f64>() < loss_probabilities[new_state]
            }
        };

        if should_drop {
            // Drop the packet by not writing anything and returning Ok with 0
            // bytes
            if let Some(event) = &*this.event {
                let _ = event
                    .on_applied(FaultEvent::PacketLoss {}, Direction::Egress);
            }
            Poll::Ready(Ok(0))
        } else {
            // Proceed to write normally
            this.inner.poll_write(cx, buf)
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.project();
        this.inner.poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.project();
        this.inner.poll_shutdown(cx)
    }
}

/// PacketLossLimitedBidirectional wraps limited reader and writer with packet
/// loss.
#[pin_project]
struct PacketLossLimitedBidirectional<R, W> {
    #[pin]
    reader: R,
    #[pin]
    writer: W,
}

impl<R, W> PacketLossLimitedBidirectional<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
{
    /// Creates a new PacketLossLimitedBidirectional.
    fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> AsyncRead for PacketLossLimitedBidirectional<R, W>
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

impl<R, W> AsyncWrite for PacketLossLimitedBidirectional<R, W>
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
pub struct PacketLossInjector {
    pub options: PacketLossOptions,
}

impl PacketLossInjector {
    /// Creates a new PacketLossFaultInjector.
    ///
    /// # Arguments
    ///
    /// * `options` - The packet loss options.
    pub fn new(options: PacketLossOptions) -> Self {
        PacketLossInjector { options }
    }
}

#[async_trait]
impl FaultInjector for PacketLossInjector {
    /// Injects packet loss into a bidirectional stream based on the direction.
    fn inject(
        &self,
        stream: Box<dyn Bidirectional + 'static>,
        direction: &Direction,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Box<dyn Bidirectional + 'static> {
        let (read_half, write_half) = split(stream);

        let _ = event.with_fault(FaultEvent::PacketLoss {}, direction.clone());

        // Notify the event handler about the applied packet loss fault
        if direction.is_ingress() {
            let _ =
                event.on_applied(FaultEvent::PacketLoss {}, Direction::Ingress);
        }

        if direction.is_egress() {
            let _ =
                event.on_applied(FaultEvent::PacketLoss {}, Direction::Egress);
        }

        // Wrap the read half if ingress or both directions are specified
        let limited_read: Box<dyn AsyncRead + Unpin + Send> =
            if direction.is_ingress() {
                Box::new(PacketLossLimitedRead::new(
                    read_half,
                    self.options.clone(),
                    Some(event.clone()),
                ))
            } else {
                Box::new(read_half) as Box<dyn AsyncRead + Unpin + Send>
            };

        // Wrap the write half if egress or both directions are specified
        let limited_write: Box<dyn AsyncWrite + Unpin + Send> =
            if direction.is_egress() {
                Box::new(PacketLossLimitedWrite::new(
                    write_half,
                    self.options.clone(),
                    Some(event.clone()),
                ))
            } else {
                Box::new(write_half) as Box<dyn AsyncWrite + Unpin + Send>
            };

        // Combine the limited read and write into a new bidirectional stream
        let limited_bidirectional =
            PacketLossLimitedBidirectional::new(limited_read, limited_write);

        Box::new(limited_bidirectional)
    }

    /// Applies packet loss to a request builder.
    async fn apply_on_request_builder(
        &self,
        builder: reqwest::ClientBuilder,
        _event: Box<dyn ProxyTaskEvent>,
    ) -> Result<reqwest::ClientBuilder, ProxyError> {
        // No modifications needed for the request builder in this fault
        // injector
        Ok(builder)
    }

    /// Applies packet loss to an outgoing request.
    async fn apply_on_request(
        &self,
        request: ReqwestRequest,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<ReqwestRequest, ProxyError> {
        let _ = event.with_fault(FaultEvent::PacketLoss {}, Direction::Egress);

        let original_body = request.body();
        if let Some(body) = original_body {
            if let Some(bytes) = body.as_bytes() {
                let owned_bytes = Cursor::new(bytes.to_vec());

                // Wrap the owned bytes with PacketLossLimitedRead
                let packet_loss_limited_read = PacketLossLimitedRead::new(
                    owned_bytes,
                    self.options.clone(),
                    Some(event.clone()),
                );

                // Convert the PacketLossLimitedRead into a ReaderStream
                let reader_stream = ReaderStream::new(packet_loss_limited_read);

                // Replace the request body with the limited stream
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

    /// Applies packet loss to an incoming response.
    async fn apply_on_response(
        &self,
        resp: http::Response<Vec<u8>>,
        event: Box<dyn ProxyTaskEvent>,
    ) -> Result<http::Response<Vec<u8>>, ProxyError> {
        let _ = event.with_fault(FaultEvent::PacketLoss {}, Direction::Ingress);

        // Split the response into parts and body
        let (parts, body) = resp.into_parts();
        let version = parts.version;
        let status = parts.status;
        let headers = parts.headers.clone();

        // Convert the body into an owned Cursor<Vec<u8>>
        let owned_body = Cursor::new(body);

        // Wrap the owned body with PacketLossLimitedRead
        let packet_loss_limited_read = PacketLossLimitedRead::new(
            owned_body,
            self.options.clone(),
            Some(event.clone()),
        );

        // Convert the PacketLossLimitedRead into a ReaderStream
        let reader_stream = ReaderStream::new(packet_loss_limited_read);

        // Read the limited stream into a buffer
        let mut buffer = BytesMut::new();
        tokio::pin!(reader_stream);
        while let Some(chunk) = reader_stream.next().await {
            buffer.extend_from_slice(&chunk?);
        }
        let response_body = buffer.to_vec();

        // Reconstruct the HTTP response with the limited body
        let mut new_response = http::Response::new(response_body);
        *new_response.version_mut() = version;
        *new_response.status_mut() = status;
        *new_response.headers_mut() = headers;

        Ok(new_response)
    }
}
