use crate::{
    ctx::ServiceContext,
    msg::{ErrorAction, ErrorHandler, Handler, Message, StreamHandler},
    service::Service,
};
use futures::future::BoxFuture;
use std::marker::PhantomData;
use tokio::sync::oneshot;

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
    /// Proxy for handling different message envelope types
    /// under a single type. Result of this function is the
    /// action which the service should take next
    ///
    /// `service` The service to execute the proxy on
    /// `ctx`     The service context
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a>;
}

/// Wrapping structure for including the response
/// sender for a message and allowing it to implement
/// the proxy type
pub(crate) struct Envelope<M: Message> {
    /// The actual message wrapped in this envelope
    msg: M,
    /// Sender present if the envelope is waiting for
    /// a response
    tx: Option<oneshot::Sender<M::Response>>,
}

impl<M: Message> Envelope<M> {
    pub(crate) fn new(msg: M, tx: Option<oneshot::Sender<M::Response>>) -> Box<Envelope<M>> {
        Box::new(Envelope { msg, tx })
    }
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
pub(crate) struct StreamEnvelope<M> {
    /// The actual message wrapped in this envelope
    msg: M,
}

impl<M> StreamEnvelope<M> {
    pub fn new(msg: M) -> Box<StreamEnvelope<M>> {
        Box::new(StreamEnvelope { msg })
    }
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

/// Trait implemented by an action which can be executed on
/// a service and its service context and produce a response
pub trait ServiceExecutor<S: Service>: Send {
    /// The result value produced from this execution
    type Response: Sized + Send + 'static;

    /// Executor for executing some logic on the service and
    /// its context
    ///
    /// `service` The service being executed on
    /// `ctx`     The context of the service
    fn execute<'a>(self, service: &'a mut S, ctx: &'a mut ServiceContext<S>) -> Self::Response;
}

/// Executor implementation for using functions as the execution
impl<F, S, R> ServiceExecutor<S> for F
where
    S: Service,
    for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send,
    R: Sized + Send + 'static,
{
    type Response = R;

    fn execute<'a>(self, service: &'a mut S, ctx: &'a mut ServiceContext<S>) -> Self::Response {
        self(service, ctx)
    }
}

/// Envelope for wrapping service executors to allow them
/// to be sent to services
pub(crate) struct ExecutorEnvelope<S, E>
where
    S: Service,
    E: ServiceExecutor<S>,
{
    /// Action to execute on the actor
    action: E,

    /// Sender present if the envelope is waiting for
    /// a response
    tx: Option<oneshot::Sender<E::Response>>,
}

impl<S, E> ExecutorEnvelope<S, E>
where
    S: Service,
    E: ServiceExecutor<S>,
{
    pub(crate) fn new(
        action: E,
        tx: Option<oneshot::Sender<E::Response>>,
    ) -> Box<ExecutorEnvelope<S, E>> {
        Box::new(ExecutorEnvelope { action, tx })
    }
}

impl<S, E> EnvelopeProxy<S> for ExecutorEnvelope<S, E>
where
    S: Service,
    E: ServiceExecutor<S>,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let res = self.action.execute(service, ctx);
        if let Some(tx) = self.tx {
            tx.send(res).ok();
        }
        ServiceAction::Continue
    }
}

/// Producer which takes mutable access to the service and its
/// context and produces a future which makes use of them
pub trait AsyncProducer<S: Service>: Send {
    /// Function for producing the actual future
    fn produce<'a>(self, service: &'a mut S, ctx: &'a mut ServiceContext<S>) -> BoxFuture<'a, ()>;
}

impl<S, F> AsyncProducer<S> for F
where
    S: Service,
    for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, ()> + Send,
{
    fn produce<'a>(self, service: &'a mut S, ctx: &'a mut ServiceContext<S>) -> BoxFuture<'a, ()> {
        self(service, ctx)
    }
}

pub(crate) struct AsyncEnvelope<S, P>
where
    S: Service,
    P: AsyncProducer<S>,
{
    /// Action to execute on the actor
    producer: P,

    /// Marker for the service type
    _marker: PhantomData<S>,
}

impl<S, P> AsyncEnvelope<S, P>
where
    S: Service,
    P: AsyncProducer<S>,
{
    pub(crate) fn new(action: P) -> Box<AsyncEnvelope<S, P>> {
        Box::new(AsyncEnvelope {
            producer: action,
            _marker: PhantomData,
        })
    }
}

impl<S, P> EnvelopeProxy<S> for AsyncEnvelope<S, P>
where
    S: Service,
    P: AsyncProducer<S>,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let fut = self.producer.produce(service, ctx);
        ServiceAction::Execute(fut)
    }
}

pub(crate) struct ErrorEnvelope<M> {
    /// The actual message wrapped in this envelope
    error: M,
}

impl<M> ErrorEnvelope<M> {
    pub fn new(error: M) -> Box<ErrorEnvelope<M>> {
        Box::new(ErrorEnvelope { error })
    }
}

impl<S, M> EnvelopeProxy<S> for ErrorEnvelope<M>
where
    S: Service + ErrorHandler<M>,
    M: Send + 'static,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        match service.handle(self.error, ctx) {
            ErrorAction::Continue => ServiceAction::Continue,
            ErrorAction::Stop => ServiceAction::Stop,
        }
    }
}

/// Enevelope message used internally for triggering
/// a service stop
pub(crate) struct StopEnvelope;

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
