use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use serde::Deserialize;
use serde::Serialize;
use serde::ser::Serializer;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::Sender;
use tokio::sync::broadcast::error::SendError;

use crate::types::Direction;
use crate::types::StreamSide;

#[derive(Debug, Clone)]
pub enum TaskProgressEvent {
    Started {
        id: TaskId,
        ts: Instant,
        url: String,
    },
    WithFault {
        id: TaskId,
        ts: Instant,
        fault: FaultEvent,
    },
    IpResolved {
        id: TaskId,
        ts: Instant,
        domain: String,
        time_taken: f64,
    },
    FaultApplied {
        id: TaskId,
        ts: Instant,
        fault: FaultEvent,
    },
    TTFB {
        id: TaskId,
        ts: Instant,
    },
    ResponseReceived {
        id: TaskId,
        ts: Instant,
        status_code: u16,
    },
    Completed {
        id: TaskId,
        ts: Instant,
        time_taken: Duration,
        from_downstream_length: u64,
        from_upstream_length: u64,
    },
    Error {
        id: TaskId,
        ts: Instant,
        error: String,
    },
}

pub type TaskId = usize;
pub type TaskProgressSender = Sender<TaskProgressEvent>;
pub type TaskProgressReceiver = Receiver<TaskProgressEvent>;

pub trait ProxyTaskEvent: Send + Sync + std::fmt::Debug {
    fn on_started(
        &self,
        url: String,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn with_fault(
        &self,
        fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_resolved(
        &self,
        domain: String,
        time_taken: f64,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_completed(
        &self,
        time_taken: Duration,
        from_downstream_length: u64,
        from_upstream_length: u64,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_first_byte(
        &self,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_applied(
        &self,
        fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_response(
        &self,
        status_code: u16,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn on_error(
        &self,
        error: Box<dyn Error>,
    ) -> Result<(), SendError<TaskProgressEvent>>;

    fn clone_me(&self) -> Box<dyn ProxyTaskEvent>;
}

impl Clone for Box<dyn ProxyTaskEvent> {
    fn clone(&self) -> Box<dyn ProxyTaskEvent> {
        self.clone_me()
    }
}

#[derive(Clone, Debug)]
pub struct FaultTaskEvent {
    id: TaskId,
    sender: TaskProgressSender,
}

impl ProxyTaskEvent for FaultTaskEvent {
    fn on_started(
        &self,
        url: String,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent =
            TaskProgressEvent::Started { id: self.id, ts: Instant::now(), url };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn with_fault(
        &self,
        fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::WithFault {
            id: self.id,
            ts: Instant::now(),
            fault,
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn on_resolved(
        &self,
        domain: String,
        time_taken: f64,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::IpResolved {
            id: self.id,
            ts: Instant::now(),
            domain,
            time_taken,
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn on_completed(
        &self,
        time_taken: Duration,
        from_downstream_length: u64,
        from_upstream_length: u64,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::Completed {
            id: self.id,
            ts: Instant::now(),
            time_taken,
            from_downstream_length,
            from_upstream_length,
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn on_first_byte(
        &self,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::TTFB {
            id: self.id,
            ts: Instant::now(),
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn on_applied(
        &self,
        fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::FaultApplied {
            id: self.id,
            ts: Instant::now(),
            fault,
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn on_response(
        &self,
        status_code: u16,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::ResponseReceived {
            id: self.id,
            ts: Instant::now(),
            status_code,
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }

    fn clone_me(&self) -> Box<dyn ProxyTaskEvent> {
        Box::new(self.clone())
    }

    fn on_error(
        &self,
        error: Box<dyn Error>,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        let event: TaskProgressEvent = TaskProgressEvent::Error {
            id: self.id,
            ts: Instant::now(),
            error: error.to_string(),
        };
        let sender = self.sender.clone();
        let _ = sender.send(event);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PassthroughTaskEvent {
    id: TaskId,
    sender: TaskProgressSender,
}

impl ProxyTaskEvent for PassthroughTaskEvent {
    fn on_started(
        &self,
        _url: String,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn with_fault(
        &self,
        _fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn on_resolved(
        &self,
        _domain: String,
        _time_taken: f64,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn on_completed(
        &self,
        _time_taken: Duration,
        _from_downstream_length: u64,
        _from_upstream_length: u64,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn on_first_byte(
        &self,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn on_applied(
        &self,
        _fault: FaultEvent,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn on_response(
        &self,
        _status_code: u16,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        Ok(())
    }

    fn clone_me(&self) -> Box<dyn ProxyTaskEvent> {
        Box::new(self.clone())
    }

    fn on_error(
        &self,
        error: Box<dyn Error>,
    ) -> Result<(), SendError<TaskProgressEvent>> {
        tracing::error!("Tracing error in bypass mode: {}", error);
        Ok(())
    }
}

pub struct TaskManager {
    counter: AtomicUsize,
    pub sender: TaskProgressSender,
}

impl TaskManager {
    pub fn new(capacity: usize) -> (Arc<Self>, TaskProgressReceiver) {
        let (sender, receiver) = broadcast::channel(capacity);
        (
            Arc::new(TaskManager { counter: AtomicUsize::new(1), sender }),
            receiver,
        )
    }

    pub fn get_sender(&self) -> TaskProgressSender {
        self.sender.clone()
    }

    pub fn next_id(&self) -> TaskId {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn new_fault_event(
        &self,
        _url: String,
    ) -> Result<Box<dyn ProxyTaskEvent>, SendError<TaskProgressEvent>> {
        let event_id = self.next_id();
        Ok(Box::new(FaultTaskEvent { id: event_id, sender: self.get_sender() }))
    }

    pub async fn new_passthrough_event(
        &self,
        _url: String,
    ) -> Result<Box<dyn ProxyTaskEvent>, SendError<TaskProgressEvent>> {
        let event_id = self.next_id();
        Ok(Box::new(PassthroughTaskEvent {
            id: event_id,
            sender: self.get_sender(),
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FaultEvent {
    Latency {
        direction: Direction,
        side: StreamSide,

        #[serde(serialize_with = "serialize_duration_as_millis_f64")]
        delay: Option<Duration>,
    },
    Dns {
        direction: Direction,
        side: StreamSide,
        triggered: Option<bool>,
    },
    Bandwidth {
        direction: Direction,
        side: StreamSide,
        bps: Option<usize>,
    },
    Jitter {
        direction: Direction,
        side: StreamSide,
        #[serde(serialize_with = "serialize_duration_as_millis_f64")]
        amplitude: Option<Duration>,
        frequency: Option<f64>,
    },
    PacketLoss {
        direction: Direction,
        side: StreamSide,
    },
    HttpResponseFault {
        direction: Direction,
        side: StreamSide,
        status_code: u16,
        response_body: Option<String>,
    },
}

impl FaultEvent {
    pub fn event_type(&self) -> String {
        match self {
            FaultEvent::Latency { direction:_, side: _, delay: _ } => "latency".to_string(),
            FaultEvent::Dns { direction:_, side: _, triggered: _ } => "dns".to_string(),
            FaultEvent::Bandwidth { direction:_, side: _, bps: _ } => "bandwidth".to_string(),
            FaultEvent::Jitter { direction:_, side: _, amplitude: _, frequency: _ } => {
                "jitter".to_string()
            }
            FaultEvent::PacketLoss {direction:_, side: _} => "packetloss".to_string(),
            FaultEvent::HttpResponseFault {
                direction:_, 
                side: _, 
                status_code: _,
                response_body: _,
            } => "httperror".to_string(),
        }
    }
}

/// Helper function to serialize `Duration` as `f64` milliseconds using
/// `as_millis_f64()`.
fn serialize_duration_as_millis_f64<S>(
    duration: &Option<Duration>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Utilize the new `as_millis_f64` method
    match duration {
        Some(d) => serializer.serialize_f64(d.as_millis_f64()),
        None => serializer.serialize_none(),
    }
}
