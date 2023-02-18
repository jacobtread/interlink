use crate::{
    ctx::ServiceContext,
    envelope::{Envelope, ExecutorEnvelope, FutureEnvelope, ServiceMessage, StopEnvelope},
    msg::{Handler, Message},
    service::Service,
};
use futures::future::BoxFuture;
use tokio::{
    sync::{mpsc, oneshot},
    task::spawn_local,
};

/// Links are used to send and receive messages from services
/// you will receive a link when you start a service or through
/// the `link()` fn on a service context
///
/// Links are cheaply clonable and can be passed between threads
pub struct Link<S>(pub(crate) mpsc::UnboundedSender<ServiceMessage<S>>);

/// Alternative type to a link rather than representing a
/// service type this represents a link to a service that
/// accepts a specific message type
///
/// These are cheaply clonable
pub struct MessageLink<M: Message>(Box<dyn MessageLinkTx<M>>);

/// Sender trait implemented by types that can be used to
/// send messages of a speicifc type implements a cloning
/// impl aswell
trait MessageLinkTx<M: Message> {
    /// Sends a message using the underlying channel as an
    /// envelope with the provided `tx` value for handling
    /// responses
    fn tx(&self, msg: M, tx: Option<oneshot::Sender<M::Response>>) -> LinkResult<()>;

    /// Boxed cloning implementation which produces a cloned
    /// value without exposing a sized type
    fn boxed_clone(&self) -> Box<dyn MessageLinkTx<M>>;
}

impl<S, M> MessageLinkTx<M> for mpsc::UnboundedSender<ServiceMessage<S>>
where
    S: Service + Handler<M>,
    M: Message,
{
    fn tx(&self, msg: M, tx: Option<oneshot::Sender<M::Response>>) -> LinkResult<()> {
        self.send(Envelope::new(msg, tx))
            .map_err(|_| LinkError::Send)
    }

    fn boxed_clone(&self) -> Box<dyn MessageLinkTx<M>> {
        Box::new(self.clone())
    }
}

impl<M> MessageLink<M>
where
    M: Message,
{
    #[must_use = "Response will not be receieved unless awaited"]
    pub async fn send(&self, msg: M) -> LinkResult<M::Response> {
        let (tx, rx) = oneshot::channel();
        self.0.tx(msg, Some(tx))?;
        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_send(&self, msg: M) -> LinkResult<()> {
        self.0.tx(msg, None)
    }
}

/// Clone implementation to clone inner sender for the link
impl<S> Clone for Link<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
/// Clone implementation to clone inner sender for the recipient
impl<M: Message> Clone for MessageLink<M> {
    fn clone(&self) -> Self {
        Self(self.0.boxed_clone())
    }
}

/// Errors that can occur while working with a link
#[derive(Debug)]
pub enum LinkError {
    /// Failed to send message to service
    Send,
    /// Failed to receive response back from service
    Recv,
}

pub type LinkResult<T> = Result<T, LinkError>;

impl<S> Link<S>
where
    S: Service,
{
    /// Creates a message link type from this link type this allows you
    /// to have links to multiple different services that accept a
    /// specific message type
    pub fn message_link<M>(&self) -> MessageLink<M>
    where
        M: Message,
        S: Handler<M>,
    {
        MessageLink(self.0.boxed_clone())
    }

    /// Internal wrapper for sending service messages and handling
    /// the error responses
    pub(crate) fn tx(&self, value: ServiceMessage<S>) -> LinkResult<()> {
        match self.0.send(value) {
            Ok(_) => Ok(()),
            Err(_) => Err(LinkError::Send),
        }
    }

    /// Tells the service to complete and wait on the action which
    /// produce a future depending on the service and context. While
    /// the action is being awaited messages will not be accepted. The
    /// result of the action will be returned.  
    ///
    /// Mutable access to the service and the service context are
    /// provided to the closure
    ///
    ///
    /// ```
    /// use interlink::prelude::*;
    /// use std::time::Duration;
    /// use tokio::time::sleep;
    ///
    /// struct MyService;
    ///
    /// impl Service for MyService {}
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link: Link<MyService> = MyService {}.start();
    ///     let value = link.wait(|service, ctx| Box::pin(async move {
    ///         println!("Service waiting on processing loop")
    ///         sleep(Duration::from_millis(1000)).await;
    ///         println!("Action executed on service");
    ///         12
    ///     }))
    ///     .await
    ///     .unwrap();
    ///
    ///     assert_eq!(value, 12);
    /// }
    ///
    /// ```
    #[must_use = "Response will not be receieved unless awaited"]
    pub async fn wait<F, R>(&self, action: F) -> LinkResult<R>
    where
        for<'a> F:
            FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, R> + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.tx(FutureEnvelope::new(action, Some(tx)))?;
        rx.await.map_err(|_| LinkError::Recv)
    }

    /// Tells the service to complete and wait on the action which
    /// produce a future depending on the service and context. While
    /// the action is being awaited messages will not be accepted.
    ///
    /// Mutable access to the service and the service context are
    /// provided to the closure
    ///
    /// ```
    /// use interlink::prelude::*;
    /// use std::time::Duration;
    /// use tokio::time::sleep;
    ///
    /// struct MyService;
    ///
    /// impl Service for MyService {}
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link: Link<MyService> = MyService {}.start();
    ///     link.do_wait(|service, ctx| Box::pin(async move {
    ///         println!("Service waiting on processing loop")
    ///         sleep(Duration::from_millis(1000)).await;
    ///         println!("Action executed on service");
    ///     }))
    ///     .unwrap();
    /// }
    ///
    /// ```
    pub fn do_wait<F, R>(&self, action: F) -> LinkResult<()>
    where
        for<'a> F:
            FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, R> + Send + 'static,
        R: Send + 'static,
    {
        self.tx(FutureEnvelope::new(action, None))
    }

    /// Sends a message to the service. The service must implement a
    /// Handler for the message. Will return the response value from
    /// the handler once awaited
    ///
    /// ```
    ///
    /// use interlink::prelude::*;
    ///
    /// struct Test;
    ///
    /// impl Service for Test {}
    ///
    /// struct MyMessage {
    ///     value: String,
    /// }
    ///
    /// impl Message for MyMessage {
    ///     /// The response type the handler will use
    ///     type Response = String;
    /// }
    ///
    /// impl Handler<MyMessage> for Test {
    ///     fn handle(&mut self, msg: MyMessage, ctx: &mut ServiceContext<Self>) -> String{
    ///         msg.value
    ///     }
    /// }
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link = Test {}.start();
    ///     let resp = link.send(MyMessage {
    ///         value: "Test123".to_string()
    ///     })
    ///     .await
    ///     .unwrap();
    ///
    ///     assert_eq!(&resp, "Test123")
    /// }
    /// ```
    #[must_use = "Response will not be receieved unless awaited"]
    pub async fn send<M>(&self, msg: M) -> LinkResult<M::Response>
    where
        M: Message,
        S: Handler<M>,
    {
        let (tx, rx) = oneshot::channel();
        self.tx(Envelope::new(msg, Some(tx)))?;
        rx.await.map_err(|_| LinkError::Recv)
    }

    /// Sends a message to the service. The service must implement a
    /// Handler for the message. Will not wait for a response from
    /// the service
    ///
    /// ```
    ///
    /// use interlink::prelude::*;
    ///
    /// struct Test;
    ///
    /// impl Service for Test {}
    ///
    /// struct MyMessage {
    ///     value: String,
    /// }
    ///
    /// impl Message for MyMessage {
    ///     /// The response type the handler will use
    ///     type Response = ();
    /// }
    ///
    /// impl Handler<MyMessage> for Test {
    ///     fn handle(&mut self, msg: MyMessage, ctx: &mut ServiceContext<Self>) {
    ///         assert_eq!(&msg.value, "Test123");
    ///     }
    /// }
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link = Test {}.start();
    ///     link.do_send(MyMessage {
    ///         value: "Test123".to_string()
    ///     })
    ///     .unwrap();
    /// }
    /// ```
    pub fn do_send<M>(&self, msg: M) -> LinkResult<()>
    where
        M: Message,
        S: Handler<M>,
    {
        self.tx(Envelope::new(msg, None))
    }

    /// Executes the provided action on the service and service context
    /// awaiting the promise from this function will result in the return
    /// value of the closure. The provided closure is given mutable access
    /// to the service and context
    ///
    /// ```
    /// use interlink::prelude::*;
    ///
    /// struct Test {
    ///     value: String
    /// }
    ///
    /// impl Service for Test {}
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link = Test { value: "Test".to_string() }.start();
    ///     
    ///     let value = link.exec(|service: &mut Test, _ctx| {
    ///         service.value.push('A');
    ///
    ///         service.value.clone()
    ///     })
    ///     .await
    ///     .expect("Failed to execute action on service");
    ///
    ///     assert_eq!(value, "TestA");
    /// }
    ///
    /// ```
    #[must_use = "Response will not be receieved unless awaited"]
    pub async fn exec<F, R>(&self, action: F) -> LinkResult<R>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.tx(ExecutorEnvelope::new(action, Some(tx)))?;

        rx.await.map_err(|_| LinkError::Recv)
    }

    /// Executes the provided action on the service and service context
    /// ignoring the result of the action. The provided closure is given
    /// mutable access to the service and context
    ///
    /// ```
    /// use interlink::prelude::*;
    ///
    /// struct Test {
    ///     value: String
    /// }
    ///
    /// impl Service for Test {}
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///     let link = Test { value: "Test".to_string() }.start();
    ///     
    ///     link.do_exec(|service: &mut Test, _ctx| {
    ///         println!("Value: {}", service.value);
    ///
    ///         service.value.push('A');
    ///
    ///         println!("Value: {}", service.value);
    ///     })
    ///     .expect("Failed to execute action on service");
    /// }
    ///
    /// ```
    pub fn do_exec<F, R>(&self, action: F) -> LinkResult<()>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.tx(ExecutorEnvelope::new(action, None))
    }

    /// Tells the associated service to stop processing messages. After
    /// this message is recieved no more messages will be processed.
    pub fn stop(&self) {
        // Send the stop message to the service
        self.tx(Box::new(StopEnvelope)).ok();
    }
}
