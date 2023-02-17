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

/// Handler for accepting streams of messages for a service
/// from streams attached to the service
pub trait StreamHandler<M: Send + 'static>: Service {
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>);
}

/// Handler for accepting streams of messages for a service
/// from streams attached to the service
pub trait ErrorHandler<M: Send + 'static>: Service {
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>) -> ErrorAction {
        ErrorAction::Continue
    }
}

pub enum ErrorAction {
    Continue,
    Stop,
}
