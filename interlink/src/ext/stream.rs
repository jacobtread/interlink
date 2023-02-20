//! Module containing logic and context extensions
//! for working with streams
use crate::{
    envelope::StreamEnvelope,
    link::Link,
    msg::StreamHandler,
    service::{Service, ServiceContext},
};
use futures_util::stream::{Stream, StreamExt};

impl<S> ServiceContext<S>
where
    S: Service,
{
    /// Attaches a streaming reciever to the service context
    ///
    /// implement the StreamHandler trait on your service
    /// with the item as the Item type of the provided stream
    /// in order to handle accepting items from the stream
    ///
    /// `stream` The stream to accept from
    /// `stop`   Whether to stop the main service when this stream service ends
    pub fn attach_stream<St>(&self, stream: St, stop: bool)
    where
        S: StreamHandler<St::Item>,
        St: Stream + Send + Unpin + 'static,
        St::Item: Send + 'static,
    {
        StreamService::start(stream, self.link(), stop)
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
    /// Starts a new stream service
    ///
    /// `stream` The stream to accept items from
    /// `link`   Link to the service that will handle the items
    /// `stop`   If true the linked service will be stopped when there are no more items
    pub(crate) fn start(stream: St, link: Link<S>, stop: bool) {
        let service = StreamService { stream, link, stop };
        tokio::spawn(service.process());
    }

    /// Processing loop for handling incoming messages from the
    /// underlying stream and forwarding them on within stream
    /// envelopes to the linked service
    async fn process(mut self) {
        while let Some(msg) = self.stream.next().await {
            if self.link.tx(StreamEnvelope::new(msg)).is_err() {
                // Linked service has ended stop processing
                return;
            }
        }

        if self.stop {
            // Stop the linked service because there are no more items
            self.link.stop();
        }
    }
}
