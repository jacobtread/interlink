use crate::{
    ctx::ServiceContext,
    envelope::{Envelope, ExecutorEnvelope, FutureEnvelope, ServiceMessage, StopEnvelope},
    msg::{Handler, Message},
    service::Service,
};
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};

/// Links are used to send and receive messages from services
/// you will receive a link when you start a service or through
/// the `link()` fn on a service context
///
/// Links are cheaply clonable and can be passed between threads
pub struct Link<S>(pub(crate) mpsc::UnboundedSender<ServiceMessage<S>>);

/// Clone implementation to clone inner sender for the link
impl<S: Service> Clone for Link<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
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
    /// the action is being awaited messages will not be accepted
    ///
    ///
    /// ```
    /// use interlink::service::Service;
    ///
    /// struct MyService;
    ///
    /// impl Service for MyService {}
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let link = MyService {}.start();
    ///     link.do_wait(|service, ctx| Box::pin(async move {
    ///     
    ///     }))
    ///     .unwrap();
    /// }
    ///
    ///
    pub fn do_wait<F, R>(&self, action: F) -> LinkResult<()>
    where
        for<'a> F:
            FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, R> + Send + 'static,
        R: Send + 'static,
    {
        self.tx(FutureEnvelope::new(action, None))
    }

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

    pub async fn send<M>(&self, msg: M) -> LinkResult<M::Response>
    where
        M: Message,
        S: Handler<M>,
    {
        let (tx, rx) = oneshot::channel();
        self.tx(Envelope::new(msg, Some(tx)))?;
        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_send<M>(&self, msg: M) -> LinkResult<()>
    where
        M: Message,
        S: Handler<M>,
    {
        self.tx(Envelope::new(msg, None))
    }

    pub async fn exec<F, R>(&self, action: F) -> LinkResult<R>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.tx(ExecutorEnvelope::new(action, Some(tx)))?;

        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_exec<F, R>(&self, action: F) -> LinkResult<()>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.tx(ExecutorEnvelope::new(action, None))
    }

    pub fn stop(&self) {
        // Send the stop message to the service
        self.tx(Box::new(StopEnvelope)).ok();
    }
}
