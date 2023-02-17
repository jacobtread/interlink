use crate::{ctx::ServiceContext, service::Service};

/// Message type implemented by structures that can be passed
/// around as messages through envelopes
pub trait Message: Send + 'static {
    /// The type of the response that handlers will produce
    /// when handling this message
    type Response: Sized + Send + 'static;
}

/// Handler implementation for allowing a service to handle a specific
/// message type
pub trait Handler<M: Message>: Service {
    /// Handler for processing the message using the current service
    /// context and message
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>) -> M::Response;
}
