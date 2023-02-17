use tokio::sync::mpsc;

use crate::{
    envelope::ServiceMessage,
    link::Link,
    service::{Service, ServiceAction},
};

/// Backing context for a service which handles storing the
/// reciever for messaging and the original link for spawning
/// copies
pub struct ServiceContext<S: Service> {
    /// Reciever for handling incoming messages for the service
    rx: mpsc::UnboundedReceiver<ServiceMessage<S>>,
    /// The original link cloned to create other links to the service
    link: Link<S>,
}

impl<S> ServiceContext<S>
where
    S: Service,
{
    pub fn new() -> ServiceContext<S> {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link { tx };

        ServiceContext { rx, link }
    }

    pub async fn process(&mut self, service: &mut S) {
        while let Some(msg) = self.rx.recv().await {
            let action = msg.handle(service, self);
            match action {
                ServiceAction::Stop => break,
                ServiceAction::Continue => continue,
                ServiceAction::Execute(fut) => fut.await,
            }
        }
    }

    pub fn link(&self) -> Link<S> {
        self.link.clone()
    }
}
