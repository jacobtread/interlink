use crate::{
    msg::{BoxFuture, ErrorAction, ErrorHandler, Handler, Message, ResponseHandler, StreamHandler},
    service::{Service, ServiceContext},
};
use std::task::ready;
use std::{future::Future, pin::Pin};
use tokio::sync::oneshot;

/// Type of a message used to communicate between services
pub(crate) type ServiceMessage<S> = Box<dyn EnvelopeProxy<S>>;

/// Actions that can be executed by the service processor
/// after its handled an action
pub(crate) enum ServiceAction<'a> {
    /// Tell service to shutdown
    Stop,
    /// Continue handling the next message
    Continue,
    /// Ask the service to execute a future on the service
    Execute(BoxFuture<'a, ()>),
}

/// Type wrapping a boxed future for storing an optional
/// oneshot sender which will be used to send the result
/// of the future once its complete
struct ExecuteFuture<'a, R> {
    fut: BoxFuture<'a, R>,
    tx: Option<oneshot::Sender<R>>,
}

impl<'a, R> ExecuteFuture<'a, R>
where
    R: Send + 'static,
{
    /// Wraps the provided future in  an execute future and
    /// returns a new boxed future
    ///
    /// `fut` The future to wrap
    /// `tx`  The optional response sender
    pub(crate) fn wrap(fut: BoxFuture<'a, R>, tx: Option<oneshot::Sender<R>>) -> BoxFuture<'a, ()> {
        Box::pin(ExecuteFuture { fut, tx })
    }
}

impl<'a, R> Future for ExecuteFuture<'a, R>
where
    R: Send,
{
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();

        // Poll the underlying future
        let result = ready!(Pin::new(&mut this.fut).poll(cx));

        // Send the response if we have a sender
        if let Some(tx) = this.tx.take() {
            let _ = tx.send(result);
        }

        std::task::Poll::Ready(())
    }
}

/// Proxy for handling the contents of a boxed envelope using the
/// provided service and service context
pub(crate) trait EnvelopeProxy<S: Service>: Send {
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

impl<S, M, R> EnvelopeProxy<S> for Envelope<M>
where
    S: Handler<M, Response = R>,
    S: Service,
    M: Message,
    R: ResponseHandler<S, M>,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let res = service.handle(self.msg, ctx);
        res.respond(service, ctx, self.tx);
        ServiceAction::Continue
    }
}

/// Trait implemented by an action which can be executed on
/// a service and its service context and produce a response
pub trait ServiceExecutor<S: Service>: Send {
    /// The result value produced from this execution
    type Response: Send;

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
    R: Send,
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
            let _ = tx.send(res);
        }
        ServiceAction::Continue
    }
}

/// Producer which takes mutable access to the service and its
/// context and produces a future which makes use of them
pub trait FutureProducer<S: Service>: Send + 'static {
    type Response: Send + 'static;

    /// Function for producing the actual future
    fn produce<'a>(
        self,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, Self::Response>;

    /// Function for producing the future while boxed
    fn produce_boxed<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, Self::Response>;
}

/// Implementation for using a closure which takes the service
/// and context to produce a box future as a async producer
impl<S, F, R> FutureProducer<S> for F
where
    S: Service,
    for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, R> + Send + 'static,
    R: Send + 'static,
{
    type Response = R;

    fn produce<'a>(
        self,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, Self::Response> {
        self(service, ctx)
    }

    fn produce_boxed<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> BoxFuture<'a, Self::Response> {
        self(service, ctx)
    }
}

/// Enevelop wrapping a boxed future producer for handling
/// executing a future on the processing loop
pub(crate) struct BoxedFutureEnvelope<S, R> {
    producer: Box<dyn FutureProducer<S, Response = R>>,
    tx: Option<oneshot::Sender<R>>,
}

impl<S, R> BoxedFutureEnvelope<S, R>
where
    S: Service,
    R: Send + 'static,
{
    pub(crate) fn new(
        producer: Box<dyn FutureProducer<S, Response = R>>,
        tx: Option<oneshot::Sender<R>>,
    ) -> Box<BoxedFutureEnvelope<S, R>> {
        Box::new(BoxedFutureEnvelope { producer, tx })
    }
}

impl<S, R> EnvelopeProxy<S> for BoxedFutureEnvelope<S, R>
where
    S: Service,
    R: Send + 'static,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let fut = self.producer.produce_boxed(service, ctx);
        ServiceAction::Execute(ExecuteFuture::wrap(fut, self.tx))
    }
}

/// Enevelop wrapping a future producer for handling
/// executing a future on the processing loop
pub(crate) struct FutureEnvelope<S, P>
where
    S: Service,
    P: FutureProducer<S>,
{
    /// Action to execute on the actor
    producer: P,

    tx: Option<oneshot::Sender<P::Response>>,
}

impl<S, P> FutureEnvelope<S, P>
where
    S: Service,
    P: FutureProducer<S>,
{
    pub(crate) fn new(
        producer: P,
        tx: Option<oneshot::Sender<P::Response>>,
    ) -> Box<FutureEnvelope<S, P>> {
        Box::new(FutureEnvelope { producer, tx })
    }
}

impl<S, P> EnvelopeProxy<S> for FutureEnvelope<S, P>
where
    S: Service,
    P: FutureProducer<S>,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        let fut = self.producer.produce(service, ctx);
        ServiceAction::Execute(ExecuteFuture::wrap(fut, self.tx))
    }
}

/// Envelope message used internally for providing messages
/// recieved from streams to their respective stream handler
pub(crate) struct StreamEnvelope<M>(M);

impl<M> StreamEnvelope<M> {
    pub fn new(msg: M) -> Box<StreamEnvelope<M>> {
        Box::new(StreamEnvelope(msg))
    }
}

impl<S, M> EnvelopeProxy<S> for StreamEnvelope<M>
where
    S: StreamHandler<M>,
    S: Service,
    M: Send,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        service.handle(self.0, ctx);
        ServiceAction::Continue
    }
}

/// Envelope message used internally for providing error
/// messages to their assocated error handler
pub(crate) struct ErrorEnvelope<M>(M);

impl<M> ErrorEnvelope<M> {
    pub(crate) fn new(error: M) -> Box<ErrorEnvelope<M>> {
        Box::new(ErrorEnvelope(error))
    }
}

impl<S, M> EnvelopeProxy<S> for ErrorEnvelope<M>
where
    S: Service + ErrorHandler<M>,
    M: Send,
{
    fn handle<'a>(
        self: Box<Self>,
        service: &'a mut S,
        ctx: &'a mut ServiceContext<S>,
    ) -> ServiceAction<'a> {
        match service.handle(self.0, ctx) {
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
