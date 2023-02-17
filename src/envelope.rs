use futures::future::BoxFuture;
use tokio::sync::oneshot;

use crate::{
    ctx::ServiceContext,
    msg::{ErrorAction, ErrorHandler, Handler, Message, StreamHandler},
    service::Service,
};

/// Type of a message used to communicate between services
pub type ServiceMessage<S> = Box<dyn EnvelopeProxy<S>>;

/// Actions that can be executed by the service processor
/// after its handled an action
pub enum ServiceAction<'a> {
    /// Tell service to shutdown
    Stop,
    /// Continue handling the next message
    Continue,
    /// Ask the service to execute a future on the service
    Execute(BoxFuture<'a, ()>),
}

/// Proxy for handling the contents of a boxed envelope using the
/// provided service and service context
pub trait EnvelopeProxy<S: Service>: Send {
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a>;
}

/// Wrapping structure for including the response
/// sender for a message and allowing it to implement
/// the proxy type
pub struct Envelope<M: Message> {
    /// The actual message wrapped in this envelope
    pub msg: M,
    /// Sender present if the envelope is waiting for
    /// a response
    pub tx: Option<oneshot::Sender<M::Response>>,
}

impl<S, M> EnvelopeProxy<S> for Envelope<M>
where
    S: Handler<M>,
    S: Service,
    M: Message,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let res = service.handle(self.msg, ctx);
        if let Some(tx) = self.tx {
            tx.send(res).ok();
        }
        ServiceAction::Continue
    }
}
pub struct StreamEnvelope<M> {
    /// The actual message wrapped in this envelope
    pub msg: M,
}

impl<S, M> EnvelopeProxy<S> for StreamEnvelope<M>
where
    S: StreamHandler<M>,
    S: Service,
    M: Send + 'static,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        service.handle(self.msg, ctx);
        ServiceAction::Continue
    }
}

pub struct ErrorEnvelope<M> {
    /// The actual message wrapped in this envelope
    pub msg: M,
}

impl<S, M> EnvelopeProxy<S> for ErrorEnvelope<M>
where
    S: ErrorHandler<M>,
    S: Service,
    M: Send + 'static,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        match service.handle(self.msg, ctx) {
            ErrorAction::Continue => ServiceAction::Continue,
            ErrorAction::Stop => ServiceAction::Stop,
        }
    }
}

pub struct ExecutorEnvelope<S, R>
where
    S: Service,
    R: Sized + Send + 'static,
{
    /// Action to execute on the actor
    pub action: Box<dyn for<'a> FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send>,
    /// Sender present if the envelope is waiting for
    /// a response
    pub tx: Option<oneshot::Sender<R>>,
}

impl<S, R> EnvelopeProxy<S> for ExecutorEnvelope<S, R>
where
    S: Service,
    R: Sized + Send + 'static,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let res = (self.action)(service, ctx);
        if let Some(tx) = self.tx {
            tx.send(res).ok();
        }
        ServiceAction::Continue
    }
}

/// Producer which takes mutable access to the service and its
/// context and produces a future which makes use of them
pub trait AsyncProducer<'a, S: Service>: Send {
    /// Function for producing the actual future
    fn produce(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, ()>;
}

impl<'a, S, F> AsyncProducer<'a, S> for F
where
    S: Service,
    F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, ()> + Send,
{
    fn produce(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, ()> {
        self(service, ctx)
    }
}

pub struct AsyncEnvelope<S>
where
    S: Service,
{
    /// Action to execute on the actor
    pub action: Box<dyn for<'a> AsyncProducer<'a, S>>,
}

impl<S> EnvelopeProxy<S> for AsyncEnvelope<S>
where
    S: Service,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let fut = self.action.produce(service, ctx);
        ServiceAction::Execute(fut)
    }
}

/// Enevelope message for triggering a service stop
pub struct StopEnvelope;

impl<S> EnvelopeProxy<S> for StopEnvelope
where
    S: Service,
{
    fn handle<'a>(
        self: Box<Self>,
        _service: &'a mut S,
        _ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        ServiceAction::Stop
    }
}
