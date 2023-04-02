//! # Messages
//!
//! Messages are used to communicate with services. Messages are handled by [`Handler`]'s on services
//! and must respond with a specific message type. The default derived message response type is the unit type.
//! When handling a value you must choose a response type for how you intent to create the response value.
//! See the different types below:
//!
//! ## Response Types
//!
//! - () Unit response type. This type responds with a empty value allowing you to return nothing from a handler
//! - [`Mr`] Message response type. This is for when you are synchronously responding to a message.
//! - [`Fr`] Future response type. This is for responding with a value that is created by awaiting a future. The future is spawned into a new tokio task
//! - [`Sfr`] Service future response type. This is for when the response depends on awaiting a future that requires a mutable borrow over the service and/or the service context
//!
//! ## Messages
//!
//! Things that can be sent to services as messages must implement the [`Message`] trait. This trait can also be
//! derived using the following derive macro.
//!
//! ```
//! use interlink::prelude::*;
//!
//! #[derive(Message)]
//! struct MyMessage {
//!     value: String,
//! }
//! ```
//!
//! Without specifying the response type in the above message it will default to the () unit response type. To specify the
//! response type you can use the syntax below
//!
//! ```
//! use interlink::prelude::*;
//!
//! #[derive(Message)]
//! #[msg(rtype = "String")]
//! struct MyMessage {
//!     value: String,
//! }
//! ```
//!
//! The rtype portion specifies the type of the response value.
//!
use std::{future::ready, pin::Pin};
use crate::{
    envelope::{BoxedFutureEnvelope, FutureProducer},
    service::{Service, ServiceContext},
};
use std::future::Future;
use tokio::sync::oneshot;

/// Type alias for a future that is pinned and boxed with a specific return type (T) and lifetime ('a)
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Message type implemented by structures that can be passed
/// around as messages through envelopes.
///
/// This trait can be derived using its derive macro
/// ```
/// use interlink::prelude::*;
///
/// #[derive(Message)]
/// struct MyMessage {
///     value: String,
/// }
/// ```
///
/// Without specifying the response type in the above message it will default to the () unit response type. To specify the
/// response type you can use the syntax below
///
/// ```
/// use interlink::prelude::*;
///
/// #[derive(Message)]
/// #[msg(rtype = "String")]
/// struct MyMessage {
///     value: String,
/// }
/// ```
pub trait Message: Send + 'static {
    /// The type of the response that handlers will produce
    /// when handling this message
    type Response: Send + 'static;
}

/// # Message Response
///
/// Response type from a handler which directly sends a response
/// for a specific message
///
/// ```
/// use interlink::prelude::*;
///
/// #[derive(Service)]
/// struct Test { value: String };
///
/// #[derive(Message)]
/// #[msg(rtype="String")]
/// struct TestMessage {
///     value: String,
/// }
///
/// impl Handler<TestMessage> for Test {
///     type Response = Mr<TestMessage>;
///
///     fn handle(&mut self, msg: TestMessage, ctx: &mut ServiceContext<Self>) -> Self::Response {
///         self.value = msg.value;
///
///         Mr("Response".to_string())
///     }
/// }
///
/// #[tokio::test]
/// async fn test() {
///     let service = Test { value: "Default".to_string() };
///     let link = service.start();
///     
///     let res: String = link
///         .send(TestMessage {
///             value: "Example".to_string()
///         })
///         .await
///         .unwrap();
///     
///     assert_eq!(&res, "Response")    
///
/// }
/// ```
pub struct Mr<M: Message>(pub M::Response);

impl<S, M> ResponseHandler<S, M> for Mr<M>
where
    S: Service,
    M: Message,
{
    fn respond(
        self,
        _service: &mut S,
        _ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<M::Response>>,
    ) {
        if let Some(tx) = tx {
            let _ = tx.send(self.0);
        }
    }
}

/// Void response handler for sending an empty unit
/// response automatically after executing
impl<S, M> ResponseHandler<S, M> for ()
where
    S: Service,
    M: Message<Response = ()>,
{
    fn respond(
        self,
        _service: &mut S,
        _ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<<M as Message>::Response>>,
    ) {
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
    }
}

/// Response handler for optional handler types to handle
/// not sending any response
impl<S, M, R> ResponseHandler<S, M> for Option<R>
where
    R: ResponseHandler<S, M>,
    S: Service,
    M: Message,
{
    fn respond(
        self,
        service: &mut S,
        ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<<M as Message>::Response>>,
    ) {
        if let Some(value) = self {
            value.respond(service, ctx, tx);
        }
    }
}

/// Response handler for result response types where the
/// error half of the result can be handled by a service
/// error handler
impl<S, M, R, E> ResponseHandler<S, M> for Result<R, E>
where
    R: ResponseHandler<S, M>,
    S: Service + ErrorHandler<E>,
    M: Message,
    E: Send + 'static,
{
    fn respond(
        self,
        service: &mut S,
        ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<<M as Message>::Response>>,
    ) {
        match self {
            Ok(value) => {
                value.respond(service, ctx, tx);
            }
            Err(err) => {
                service.handle(err, ctx);
            }
        }
    }
}

/// # Future Response
///
/// Response type from a handler containing a future that
/// is to be spawned into a another task where the response
/// will then be sent to the sender. This should be used
/// when the response is computed in a future that can run
/// independently from the service.
///
///
/// ```
/// use interlink::prelude::*;
/// use std::time::Duration;
/// use tokio::time::sleep;
///
/// #[derive(Service)]
/// struct Test { value: String };
///
/// #[derive(Message)]
/// #[msg(rtype = "String")]
/// struct TestMessage {
///     value: String,
/// }
///
/// impl Handler<TestMessage> for Test {
///     type Response = Fr<TestMessage>;
///
///     fn handle(&mut self, msg: TestMessage, ctx: &mut ServiceContext<Self>) -> Self::Response {
///         // Additional logic can be run here before the future
///         // response is created
///
///         Fr::new(Box::pin(async move {
///             // Some future that must be polled in another task
///             sleep(Duration::from_millis(1000)).await;
///
///             // You can return the response type of the message here
///             "Response".to_string()
///        }))
///     }
/// }
///
/// #[tokio::test]
/// async fn test() {
///     let service = Test { value: "Default".to_string() };
///     let link = service.start();
///     
///     let res: String = link
///         .send(TestMessage {
///             value: "Example".to_string()
///         })
///         .await
///         .unwrap();
///     
///     assert_eq!(&res, "Response")    
///
/// }
/// ```
pub struct Fr<M: Message> {
    /// The underlying future to await for a response
    future: BoxFuture<'static, M::Response>,
}

impl<M> Fr<M>
where
    M: Message,
{
    /// Creates a new future response from the provided boxed future.
    pub fn new(future: BoxFuture<'static, M::Response>) -> Fr<M> {
        Fr { future }
    }

    /// Creates a Fr wrapping the provided future creating a boxed
    /// future from the provided future. Don't use this if you've
    /// already boxed the future
    pub fn new_box<F>(future: F) -> Fr<M>
    where
        F: Future<Output = M::Response> + Send + 'static,
    {
        Fr {
            future: Box::pin(future),
        }
    }

    /// Creates a new future response for a ready future containing a value
    /// that is already ready.
    pub fn ready(value: M::Response) -> Fr<M> {
        Fr {
            future: Box::pin(ready(value)),
        }
    }
}

impl<S, M> ResponseHandler<S, M> for Fr<M>
where
    S: Service,
    M: Message,
{
    fn respond(
        self,
        _service: &mut S,
        _ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<M::Response>>,
    ) {
        tokio::spawn(async move {
            let res = self.future.await;
            if let Some(tx) = tx {
                let _ = tx.send(res);
            }
        });
    }
}

/// # Service Future Response
///
/// Response type from a handler where a future must be
/// awaited on the processing loop of the service. While
/// the result of this future is being processed no other
/// messages will be handled.
///
/// This provides a mutable borrow of the service and the service
/// context to the future that is being awaited.
///
/// ```
/// use interlink::prelude::*;
/// use std::time::Duration;
/// use tokio::time::sleep;
///
/// #[derive(Service)]
/// struct Test { value: String };
///
/// #[derive(Message)]
/// #[msg(rtype = "String")]
/// struct TestMessage {
///     value: String,
/// }
///
/// impl Handler<TestMessage> for Test {
///     type Response = Sfr<Self, TestMessage>;
///
///     fn handle(&mut self, msg: TestMessage, ctx: &mut ServiceContext<Self>) -> Self::Response {
///         // Additional logic can be run here before the future
///         // response is created
///
///         Sfr::new(move |service: &mut Test, ctx| {
///             Box::pin(async move {
///                 // Some future that must be polled on the service loop
///                 sleep(Duration::from_millis(1000)).await;
///
///                 // Make use of the mutable access to service
///                 service.value = msg.value.clone();
///
///                 // You can return the response type of the message here
///                 "Response".to_string()
///             })
///         })
///     }
/// }
///
/// #[tokio::test]
/// async fn test() {
///     let service = Test { value: "Default".to_string() };
///     let link = service.start();
///     
///     let res: String = link
///         .send(TestMessage {
///             value: "Example".to_string()
///         })
///         .await
///         .unwrap();
///     
///     assert_eq!(&res, "Response")    
///
/// }
/// ```
pub struct Sfr<S, M: Message> {
    /// The producer that will produce the function that must be awaited
    producer: Box<dyn FutureProducer<S, Response = M::Response>>,
}

impl<S, M> Sfr<S, M>
where
    S: Service,
    M: Message,
{
    /// Creates a new service future response. Takes a fn which
    /// accepts mutable access to the service and its context
    /// and returns a boxed future with the same lifetime as the
    /// borrow
    ///
    /// `producer` The producer fn
    pub fn new<P>(producer: P) -> Sfr<S, M>
    where
        for<'a> P: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, M::Response>
            + Send
            + 'static,
    {
        Sfr {
            producer: Box::new(producer),
        }
    }
}

/// The response handler for service future responses passes on
/// the producer in an envelope to be handled by the context
impl<S, M> ResponseHandler<S, M> for Sfr<S, M>
where
    S: Service,
    M: Message,
{
    fn respond(
        self,
        _service: &mut S,
        ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<M::Response>>,
    ) {
        let _ = ctx
            .shared_link()
            .tx(BoxedFutureEnvelope::new(self.producer, tx));
    }
}

/// Handler implementation for handling what happens
/// with a response value
pub trait ResponseHandler<S: Service, M: Message>: Send + 'static {
    fn respond(
        self,
        service: &mut S,
        ctx: &mut ServiceContext<S>,
        tx: Option<oneshot::Sender<M::Response>>,
    );
}

/// Handler implementation for allowing a service to handle a specific
/// message type
pub trait Handler<M: Message>: Service {
    /// The response type this handler will use
    type Response: ResponseHandler<Self, M>;

    /// Handler for processing the message using the current service
    /// context and message. Will respond with the specified response type
    ///
    /// `self` The service handling the message
    /// `msg`  The message that is being handled
    /// `ctx`  Mutable borrow of the service context
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>) -> Self::Response;
}

/// Handler for accepting streams of messages for a service
/// from streams attached to the service see `attach_stream`
/// on ServiceContext
pub trait StreamHandler<M: Send>: Service {
    /// Handler for handling messages received from a stream
    ///
    /// `self` The service handling the message
    /// `msg`  The message received
    /// `ctx`  Mutable borrow of the service context
    fn handle(&mut self, msg: M, ctx: &mut ServiceContext<Self>);
}

/// Handler for accepting streams of messages for a service
/// from streams attached to the service
pub trait ErrorHandler<M: Send>: Service {

    /// Handler for handling errors that occur in associated services
    /// in cases such as errors while writing messages to a connected sink.
    /// Responds with an [`ErrorAction`] which determines how the service
    /// should react to the error
    ///
    /// `self` The service handling the error
    /// `err`  The error that was encountered
    /// `ctx`  Mutable borrow of the service context
    fn handle(&mut self, err: M, ctx: &mut ServiceContext<Self>) -> ErrorAction;
}

/// Actions that can be taken after handling an error.
pub enum ErrorAction {
    /// Continue processing
    Continue,
    /// Stop processing
    Stop,
}
