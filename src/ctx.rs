use crate::{
    envelope::{ErrorEnvelope, ServiceAction, ServiceMessage, StreamEnvelope},
    link::{Link, LinkError},
    msg::{ErrorHandler, StreamHandler},
    service::Service,
};
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;

/// Backing context for a service which handles storing the
/// reciever for messaging and the original link for spawning
/// copies
pub struct ServiceContext<S: Service> {
    /// Reciever for handling incoming messages for the service
    rx: mpsc::UnboundedReceiver<ServiceMessage<S>>,
    /// The original link cloned to create other links to the service
    link: Link<S>,
}

impl<S> ServiceContext<S>
where
    S: Service,
{
    /// Attaches a streaming reciever to the service context
    pub fn attach_stream<St>(&self, stream: St, stop: bool)
    where
        S: StreamHandler<St::Item>,
        St: Stream + Send + Unpin + 'static,
        St::Item: Send + 'static,
    {
        StreamService::start(stream, self.link(), stop)
    }

    /// Attaches a sink to this service and provides a link to the
    /// service so that it can be used to write messages
    ///
    /// `sink` The sink to attach
    pub fn attach_sink<Si, I>(&self, sink: Si) -> SinkLink<I>
    where
        S: ErrorHandler<Si::Error>,
        Si: Sink<I> + Send + Unpin + 'static,
        Si::Error: Send + 'static,
        I: Send + 'static,
    {
        SinkService::start(sink, self.link())
    }

    /// Creates a new service context and the initial link
    pub(crate) fn new() -> ServiceContext<S> {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link(tx);

        ServiceContext { rx, link }
    }

    pub async fn process(&mut self, service: &mut S) {
        while let Some(msg) = self.rx.recv().await {
            let action = msg.handle(service, self);
            match action {
                ServiceAction::Stop => break,
                ServiceAction::Continue => continue,
                // Execute tasks that require blocking the processing
                ServiceAction::Execute(fut) => fut.await,
            }
        }
    }

    pub fn link(&self) -> Link<S> {
        self.link.clone()
    }
}

/// Service for reading items from a stream and sending the
/// items to the StreamHandler on the provided service can
/// optionally stop the provided service when there are
/// no more items.
struct StreamService<S, St> {
    /// The stream to consume
    stream: St,
    /// Link to the associated service
    link: Link<S>,
    /// Whether to stop the associated service when this stream ends
    stop: bool,
}

impl<S, St> StreamService<S, St>
where
    S: Service + StreamHandler<St::Item>,
    St: Stream + Send + Unpin + 'static,
    St::Item: Send + 'static,
{
    pub fn start(stream: St, link: Link<S>, stop: bool) {
        let service = StreamService { stream, link, stop };
        tokio::spawn(service.process());
    }

    pub async fn process(mut self) {
        while let Some(msg) = self.stream.next().await {
            if self.link.tx(StreamEnvelope::new(msg)).is_err() {
                // Assocated service has ended stop processing
                return;
            }
        }

        if self.stop {
            self.link.stop();
        }
    }
}

/// Service for handling a Sink and its writing this is
/// a lightweight service which has its own link type
/// and doesn't implement normal service logic to be more lightweight
struct SinkService<S, Si, I> {
    sink: Si,
    link: Link<S>,
    rx: mpsc::UnboundedReceiver<SinkMessage<I>>,
}

/// Dedicated link type for sinks
pub struct SinkLink<I>(mpsc::UnboundedSender<SinkMessage<I>>);

/// Messages used to communicate with the sink
enum SinkMessage<I> {
    /// Immediately write a message using the sink
    Send(I),
    /// Feed an item to be written later on flush
    Feed(I),
    /// Flush all the items in the sink
    Flush,
    /// Tells the sink to stop processing values
    Stop,
}

/// Additional logic for sink links
impl<I> SinkLink<I>
where
    I: Send,
{
    pub fn sink(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Send(item))
            .map_err(|_| LinkError::Send)
    }

    pub fn feed(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Feed(item))
            .map_err(|_| LinkError::Send)
    }

    pub fn flush(&self) -> Result<(), LinkError> {
        self.0.send(SinkMessage::Flush).map_err(|_| LinkError::Send)
    }

    pub fn stop(&self) -> Result<(), LinkError> {
        self.0.send(SinkMessage::Stop).map_err(|_| LinkError::Send)
    }
}

impl<S, Si, I> SinkService<S, Si, I>
where
    S: Service + ErrorHandler<Si::Error>,
    Si: Sink<I> + Send + Unpin + 'static,
    Si::Error: Send + 'static,
    I: Send + 'static,
{
    pub fn start(sink: Si, link: Link<S>) -> SinkLink<I> {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink_link = SinkLink(tx);
        let service = SinkService { sink, link, rx };
        tokio::spawn(service.process());
        sink_link
    }

    pub async fn process(mut self) {
        while let Some(msg) = self.rx.recv().await {
            let result = match msg {
                SinkMessage::Send(value) => self.sink.send(value).await,
                SinkMessage::Feed(value) => self.sink.feed(value).await,
                SinkMessage::Flush => self.sink.flush().await,
                SinkMessage::Stop => break,
            };

            if let Err(err) = result {
                // If the error message couldn't be sent the service is stopped
                if self.link.tx(ErrorEnvelope::new(err)).is_err() {
                    break;
                }
            }
        }
    }
}
