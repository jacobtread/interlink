use crate::{
    envelope::{EnvelopeProxy, ServiceAction, ServiceMessage},
    link::{Link, LinkError},
    msg::{ErrorHandler, StreamHandler},
    service::Service,
};
use futures::{Sink, SinkExt, Stream, StreamExt};
use std::marker::PhantomData;
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
    pub fn attach_stream<St, I>(&self, stream: St, stop: bool)
    where
        St: Stream<Item = I> + Send + Unpin + 'static,
        I: Send + 'static,
        S: StreamHandler<I>,
    {
        let proc = StreamProccessor {
            stream,
            link: self.link(),
            stop,
        };
        tokio::spawn(proc.process());
    }

    /// Attaches a sink to this service and provides a link to the
    /// service so that it can be used to write messages
    ///
    /// `sink` The sink to attach
    pub fn attach_sink<Si, I, E>(&self, sink: Si) -> SinkLink<S, Si, I, E>
    where
        S: ErrorHandler<E>,
        Si: Sink<I, Error = E> + Send + Unpin + 'static,
        I: Send + 'static,
        E: Send + 'static,
    {
        SinkService {
            link: self.link(),
            sink,
            _marker: PhantomData,
        }
        .start()
    }

    /// Creates a new service context and the initial link
    pub(crate) fn new() -> ServiceContext<S> {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link { tx };

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

pub struct StreamProccessor<S, St, I>
where
    S: Service,
    St: Stream<Item = I>,
{
    /// The stream to consume
    stream: St,
    /// Link to the associated service
    link: Link<S>,
    /// Whether to stop the associated service when this stream ends
    stop: bool,
}

impl<S, St, I> StreamProccessor<S, St, I>
where
    S: Service,
    St: Stream<Item = I> + Send + Unpin,
    I: Send + 'static,
    S: StreamHandler<I>,
{
    pub async fn process(mut self) {
        while let Some(msg) = self.stream.next().await {
            if !self.link.consume_stream(msg) {
                // Assocated service has ended stop processing
                return;
            }
        }

        if self.stop {
            self.link.stop();
        }
    }
}

/// Service for handling a Sink and its writing
pub struct SinkService<S, Si, I, E>
where
    S: Service + ErrorHandler<E>,
    Si: Sink<I, Error = E> + Send + Unpin + 'static,
    I: Send + 'static,
    E: Send + 'static,
{
    sink: Si,
    link: Link<S>,
    _marker: PhantomData<(I, E)>,
}

type SinkLink<S, Si, I, E> = Link<SinkService<S, Si, I, E>>;

/// Implement the sinker service
impl<S, Si, I, E> Service for SinkService<S, Si, I, E>
where
    S: Service + ErrorHandler<E>,
    Si: Sink<I, Error = E> + Send + Unpin + 'static,
    I: Send + 'static,
    E: Send + 'static,
{
}

/// Additional logic for sink links
impl<S, Si, I, E> Link<SinkService<S, Si, I, E>>
where
    S: Service + ErrorHandler<E>,
    Si: Sink<I, Error = E> + Send + Unpin + 'static,
    I: Send + 'static,
    E: Send + 'static,
{
    pub fn sink(&self, item: I) -> Result<(), LinkError> {
        self.tx
            .send(Box::new(SinkMessage::Send(item)))
            .map_err(|_| LinkError::Send)
    }

    pub fn feed(&self, item: I) -> Result<(), LinkError> {
        self.tx
            .send(Box::new(SinkMessage::Feed(item)))
            .map_err(|_| LinkError::Send)
    }

    pub fn flush(&self) -> Result<(), LinkError> {
        self.tx
            .send(Box::new(SinkMessage::Flush))
            .map_err(|_| LinkError::Send)
    }
}

pub enum SinkMessage<I> {
    /// Immediately write a message using the sink
    Send(I),
    /// Feed an item to be written later on flush
    Feed(I),
    /// Flush all the items in the sink
    Flush,
}

impl<'a, S, Si, I, E> EnvelopeProxy<'a, SinkService<S, Si, I, E>> for SinkMessage<I>
where
    S: Service + ErrorHandler<E>,
    Si: Sink<I, Error = E> + Send + Unpin + 'static,
    I: Send + 'static,
    E: Send + 'static,
{
    fn handle(
        self: Box<Self>,
        service: &'a mut SinkService<S, Si, I, E>,
        _ctx: &'a mut ServiceContext<SinkService<S, Si, I, E>>,
    ) -> ServiceAction<'a> {
        ServiceAction::Execute(Box::pin(async move {
            let result = match *self {
                SinkMessage::Send(value) => service.sink.send(value).await,
                SinkMessage::Feed(value) => service.sink.feed(value).await,
                SinkMessage::Flush => service.sink.flush().await,
            };

            if let Err(err) = result {
                service.link.consume_error(err);
            }
        }))
    }
}
