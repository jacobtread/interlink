use std::marker::PhantomData;

use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;

use crate::{
    envelope::{EnvelopeProxy, ServiceMessage},
    link::Link,
    message::{ErrorHandler, Handler, Message, StreamHandler},
    service::{Service, ServiceAction},
};

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

    pub fn new() -> ServiceContext<S> {
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

impl<S, Si, I, E> Service for SinkService<S, Si, I, E>
where
    S: Service + ErrorHandler<E>,
    Si: Sink<I, Error = E> + Send + Unpin + 'static,
    I: Send + 'static,
    E: Send + 'static,
{
}

pub struct SinkMessage<I>(I);

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
            if let Err(err) = service.sink.send(self.0).await {
                service.link.consume_error(err);
            }
        }))
    }
}
