use crate::{
    envelope::{ServiceAction, ServiceMessage},
    link::Link,
    service::Service,
};
use tokio::sync::mpsc;

/// Backing context for a service which handles storing the
/// reciever for messaging and the original link for spawning
/// copies
pub struct ServiceContext<S: Service> {
    /// Reciever for handling incoming messages for the service
    rx: mpsc::UnboundedReceiver<ServiceMessage<S>>,
    /// The original link cloned to create other links to the service
    pub(crate) link: Link<S>,
}

impl<S> ServiceContext<S>
where
    S: Service,
{
    /// Creates a new service context and the initial link
    pub(crate) fn new() -> ServiceContext<S> {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link(tx);

        ServiceContext { rx, link }
    }

    /// Spawns this servuce  into a new tokio task
    /// where it will then begin processing messages
    ///
    /// `service` The service
    pub(crate) fn spawn(mut self, mut service: S) {
        tokio::spawn(async move {
            service.started(&mut self);

            self.process(&mut service).await;

            service.stopping();
        });
    }

    /// Processing loop for the service handles recieved messages and
    /// executing actions from the message handle results
    ///
    /// `service` The service this context is processing for
    async fn process(&mut self, service: &mut S) {
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

    /// Stop the context directly by closing the reciever the
    /// reciever will drain any existing messages until there
    /// are none remaining
    pub fn stop(&mut self) {
        self.rx.close()
    }

    /// Creates and returns a link to the service
    pub fn link(&self) -> Link<S> {
        self.link.clone()
    }
}

/// Module containing logic for working with streams
pub mod stream {
    use crate::{
        ctx::ServiceContext, envelope::StreamEnvelope, link::Link, msg::StreamHandler,
        service::Service,
    };
    use futures::{Stream, StreamExt};

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
}

/// Module containing logic for working with sinks
pub mod sink {
    use crate::{
        ctx::ServiceContext,
        envelope::ErrorEnvelope,
        link::{Link, LinkError},
        msg::ErrorHandler,
        service::Service,
    };
    use futures::{Sink, SinkExt};
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

    /// Dedicated link type for sinks. This is cheaply clonable
    /// so you can clone it to use it in multiple places.
    pub struct SinkLink<I>(mpsc::UnboundedSender<SinkMessage<I>>);

    impl<I> Clone for SinkLink<I> {
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

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
}
