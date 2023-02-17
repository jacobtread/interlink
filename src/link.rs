use crate::{
    ctx::ServiceContext,
    envelope::{AsyncEnvelope, Envelope, ExecutorEnvelope, ServiceMessage, StopEnvelope},
    msg::{Handler, Message},
    service::Service,
};
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};

/// Links are used to send and receive messages from services
pub struct Link<S: Service> {
    /// Sender for sending messages to the connected service
    pub(crate) tx: mpsc::UnboundedSender<ServiceMessage<S>>,
}

impl<S: Service> Clone for Link<S> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

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
        match self.tx.send(value) {
            Ok(_) => Ok(()),
            Err(_) => Err(LinkError::Send),
        }
    }

    /// Tells the service to complete and wait on the action which
    /// produce a future depending on the service and context. While
    /// the action is being awaited messages will not be accepted
    pub fn wait<F>(&self, action: F) -> LinkResult<()>
    where
        for<'a> F:
            FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, ()> + Send + 'static,
    {
        self.tx(AsyncEnvelope::new(action))
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
        for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send + 'static,
        R: Sized + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.tx(ExecutorEnvelope::new(action, Some(tx)))?;

        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_exec<F, R>(&self, action: F) -> LinkResult<()>
    where
        for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send + 'static,
        R: Sized + Send + 'static,
    {
        self.tx(ExecutorEnvelope::new(action, None))?;

        Ok(())
    }

    pub fn stop(&self) {
        // Send the stop message to the service
        self.tx(Box::new(StopEnvelope)).ok();
    }
}
