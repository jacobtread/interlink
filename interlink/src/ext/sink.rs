//! Module containing logic and context extensions
//! for working with sinks
use crate::{
    envelope::ErrorEnvelope,
    link::{Link, LinkError},
    msg::ErrorHandler,
    service::{Service, ServiceContext},
};
use futures_util::sink::{Sink, SinkExt};
use tokio::sync::mpsc;

impl<S: Service> ServiceContext<S> {
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
}

/// Service for handling a Sink and its writing this is a
/// lightweight service which has its own link type and doesn't
/// implement normal service logic to be more lightweight
struct SinkService<S, Si, I> {
    sink: Si,
    link: Link<S>,
    rx: mpsc::UnboundedReceiver<SinkMessage<I>>,
}

/// Dedicated link type for sinks.
///
/// This is cheaply clonable so you can clone
/// it to use it in multiple places.
pub struct SinkLink<I>(mpsc::UnboundedSender<SinkMessage<I>>);

impl<I> Clone for SinkLink<I> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Messages used to communicate internally with the sink
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
    /// Sends a new item to the sink. This item will be written
    /// to the underlying target as soon as the previous messages
    /// are written.
    ///
    /// This does not required calling flush
    ///
    /// `item` The item to write
    pub fn sink(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Send(item))
            .map_err(|_| LinkError::Send)
    }

    /// Feeds a new item to the sink this item will not be written
    /// until flush is called.
    ///
    /// This requires flush be called at some point to send
    ///
    /// `item` The item to feed
    pub fn feed(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Feed(item))
            .map_err(|_| LinkError::Send)
    }

    /// Requests that the Sink service flush any messages that have
    /// been fed to the sink
    pub fn flush(&self) -> Result<(), LinkError> {
        self.0.send(SinkMessage::Flush).map_err(|_| LinkError::Send)
    }

    /// Requests that the sink shutdown and stop accepting new
    /// messages. This will drop the underlying write target
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
    /// Starts a new sink service. You should attach this
    ///
    /// `sink` The sink to send and feed the items into
    /// `link` Link to the service that will handle the items
    pub(crate) fn start(sink: Si, link: Link<S>) -> SinkLink<I> {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink_link = SinkLink(tx);
        let service = SinkService { sink, link, rx };
        tokio::spawn(service.process());
        sink_link
    }

    /// Processing loop for handling messages for feeding items
    /// into the sink, sending, flushing etc.
    async fn process(mut self) {
        while let Some(msg) = self.rx.recv().await {
            let result = match msg {
                SinkMessage::Send(value) => self.sink.send(value).await,
                SinkMessage::Feed(value) => self.sink.feed(value).await,
                SinkMessage::Flush => self.sink.flush().await,
                SinkMessage::Stop => break,
            };

            if let Err(err) = result {
                if self.link.tx(ErrorEnvelope::new(err)).is_err() {
                    // If the error message couldn't be sent the service is stopped
                    break;
                }
            }
        }
    }
}
