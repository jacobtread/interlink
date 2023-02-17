use std::marker::PhantomData;

use futures::{future::BoxFuture, Future};
use tokio::sync::{mpsc, oneshot};

use crate::{
    ctx::ServiceContext,
    envelope::{AsyncEnvelope, Envelope, ExecutorEnvelope, ServiceMessage, StopEnvelope},
    message::{Handler, Message},
    service::Service,
};

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

impl<S> Link<S>
where
    S: Service,
{
    /// Tells the service to complete and wait on the action which
    /// produce a future depending on the service and context. While
    /// the action is being awaited messages will not be accepted
    pub fn wait<F>(&self, action: F)
    where
        for<'a> F:
            FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> BoxFuture<'a, ()> + Send + 'static,
    {
        self.tx
            .send(Box::new(AsyncEnvelope {
                action: Box::new(action),
            }))
            .ok();
    }

    pub async fn send<M>(&self, msg: M) -> Result<M::Response, LinkError>
    where
        M: Message,
        S: Handler<M>,
    {
        let (tx, rx) = oneshot::channel();

        self.tx
            .send(Box::new(Envelope { msg, tx: Some(tx) }))
            .map_err(|_| LinkError::Send)?;

        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_send<M>(&self, msg: M) -> Result<(), LinkError>
    where
        M: Message,
        S: Handler<M>,
    {
        self.tx
            .send(Box::new(Envelope { msg, tx: None }))
            .map_err(|_| LinkError::Send)
    }

    pub async fn exec<F, R>(&self, action: F) -> Result<R, LinkError>
    where
        for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send + 'static,
        R: Sized + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.tx
            .send(Box::new(ExecutorEnvelope {
                action: Box::new(action),
                tx: Some(tx),
            }))
            .map_err(|_| LinkError::Send)?;

        rx.await.map_err(|_| LinkError::Recv)
    }

    pub fn do_exec<F, R>(&self, action: F) -> Result<(), LinkError>
    where
        for<'a> F: FnOnce(&'a mut S, &'a mut ServiceContext<S>) -> R + Send + 'static,
        R: Sized + Send + 'static,
    {
        self.tx
            .send(Box::new(ExecutorEnvelope {
                action: Box::new(action),
                tx: None,
            }))
            .map_err(|_| LinkError::Send)?;

        Ok(())
    }

    pub fn stop(&self) {
        // Send the stop message to the service
        self.tx.send(Box::new(StopEnvelope)).ok();
    }
}
