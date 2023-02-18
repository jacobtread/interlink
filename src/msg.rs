use crate::{
    ctx::ServiceContext,
    envelope::{BoxedFutureEnvelope, FutureProducer},
    service::Service,
};
use futures::future::BoxFuture;
use tokio::sync::oneshot;

/// Message type implemented by structures that can be passed
/// around as messages through envelopes
pub trait Message: Send + 'static {
    /// The type of the response that handlers will produce
    /// when handling this message
    type Response: Send + 'static;
}

/// Response type from a handler which just returns the
/// default message value
pub struct MessageResponse<M>(pub M);

impl<S, M> ResponseHandler<S, M> for MessageResponse<M::Response>
where
    S: Service,
    M: Message,
{
    fn respond(self, _ctx: &mut ServiceContext<S>, tx: Option<oneshot::Sender<M::Response>>) {
        if let Some(tx) = tx {
            tx.send(self.0).ok();
        }
    }
}

/// Void response handler for not responding
impl<S, M> ResponseHandler<S, M> for ()
where
    S: Service,
    M: Message,
{
    fn respond(
        self,
        _ctx: &mut ServiceContext<S>,
        _tx: Option<oneshot::Sender<<M as Message>::Response>>,
    ) {
    }
}

/// Response type from a handler containing a future that
/// is to be spawned into a another task where the response
/// will then be sent to the sender
pub struct FutureResponse<M: Message> {
    future: BoxFuture<'static, M::Response>,
}

impl<M> FutureResponse<M>
where
    M: Message,
{
    pub fn new(future: BoxFuture<'static, M::Response>) -> FutureResponse<M> {
        FutureResponse { future }
    }
}

impl<S, M> ResponseHandler<S, M> for FutureResponse<M>
where
    S: Service,
    M: Message,
{
    fn respond(self, _ctx: &mut ServiceContext<S>, tx: Option<oneshot::Sender<M::Response>>) {
        tokio::spawn(async move {
            let res = self.future.await;
            if let Some(tx) = tx {
                tx.send(res).ok();
            }
        });
    }
}

/// Response type from a handler where a future must be
/// awaited on the processing loop of the service
pub struct ServiceFutureResponse<S, M: Message> {
    producer: Box<dyn FutureProducer<S, Response = M::Response>>,
}

impl<S, M> ServiceFutureResponse<S, M>
where
    S: Service,
    M: Message,
{
    pub fn new<P>(producer: P) -> ServiceFutureResponse<S, M>
    where
        for<'a> P: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, M::Response>
            + Send
            + 'static,
    {
        ServiceFutureResponse {
            producer: Box::new(producer),
        }
    }
}

impl<S, M> ResponseHandler<S, M> for ServiceFutureResponse<S, M>
where
    S: Service,
    M: Message,
{
    fn respond(self, ctx: &mut ServiceContext<S>, tx: Option<oneshot::Sender<M::Response>>) {
        ctx.link
            .tx(BoxedFutureEnvelope::new(self.producer, tx))
            .ok();
    }
}

/// Handler implementation for handling what happens
/// with a response value
pub trait ResponseHandler<S: Service, M: Message>: Send + 'static {
    fn respond(self, ctx: &mut ServiceContext<S>, tx: Option<oneshot::Sender<M::Response>>);
}

/// Handler implementation for allowing a service to handle a specific
/// message type
pub trait Handler<M: Message>: Service {
    type Response: ResponseHandler<Self, M>;

    /// Handler for processing the message using the current service
    /// context and message
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>) -> Self::Response;
}

/// Handler for accepting streams of messages for a service
/// from streams attached to the service
pub trait StreamHandler<M: Send>: Service {
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>);
}

/// Handler for accepting streams of messages for a service
/// from streams attached to the service
pub trait ErrorHandler<M: Send>: Service {
    fn handle(&mut self, err: M, ctx: &mut ServiceContext<Self>) -> ErrorAction;
}

pub enum ErrorAction {
    Continue,
    Stop,
}
