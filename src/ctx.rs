use tokio::sync::mpsc;

use crate::{envelope::ServiceMessage, link::Link, service::Service};

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
    pub async fn process(&mut self, service: &mut S) {}
}
