use crate::envelope::{ServiceAction, ServiceMessage};
use crate::link::Link;
use tokio::sync::mpsc;

/// Trait implemented by structures that can be spawned as
/// services and used by the app
pub trait Service: Sized + Send + 'static {
    /// Handler called before the service starts processing messages
    ///
    /// `ctx` The service context
    fn started(&mut self, #[allow(unused)] ctx: &mut ServiceContext<Self>) {}

    /// Start an already created service and provides a link for
    /// communicating with the service
    ///
    /// ```
    /// use interlink::prelude::*;
    ///
    /// #[derive(Service)]
    /// struct MyService;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Create the service
    ///     let service: MyService = MyService {};
    ///     // Start the service and obtain a link to it
    ///     let addr: Link<MyService> = service.start();
    /// }
    /// ```
    fn start(self) -> Link<Self> {
        let ctx = ServiceContext::new();
        let link = ctx.link();
        ctx.spawn(self);
        link
    }

    /// Alternative way of creating a service where the service may
    /// rely on the context an example of this is an associated service
    /// which requires a link to the service but is also stored on the
    /// service struct
    ///
    /// ```
    /// use interlink::prelude::*;
    ///
    /// #[derive(Service)]
    /// struct First {
    ///     /// Link to spawned service
    ///     second: Link<Second>,
    /// }
    ///
    /// /// Some other service which requires a link to our service
    /// #[derive(Service)]
    /// struct Second {
    ///     /// Link to the service that owns this service
    ///     owner: Link<First>
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Provide a closure which takes in the ctx    
    ///     let link: Link<First> = First::create(|ctx| {
    ///     
    ///         // Create second which references the context
    ///         let second: Link<Second> = Second {
    ///             owner: ctx.link()
    ///         }
    ///         .start();
    ///     
    ///         // Can now use the spawned value
    ///         First { second }    
    ///     });
    /// }
    /// ```
    fn create<F>(action: F) -> Link<Self>
    where
        F: FnOnce(&mut ServiceContext<Self>) -> Self,
    {
        let mut ctx = ServiceContext::new();
        let service = action(&mut ctx);
        let link = ctx.link();
        ctx.spawn(service);
        link
    }

    /// Handler logic called when the service is stopping
    fn stopping(&mut self) {}
}

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
    /// Creates a new service context and the initial link
    pub(crate) fn new() -> ServiceContext<S> {
        let (tx, rx) = mpsc::unbounded_channel();
        let link = Link(tx);

        ServiceContext { rx, link }
    }

    /// Spawns this servuce  into a new tokio task
    /// where it will then begin processing messages
    ///
    /// `service` The service
    pub(crate) fn spawn(mut self, mut service: S) {
        tokio::spawn(async move {
            service.started(&mut self);

            self.process(&mut service).await;

            service.stopping();
        });
    }

    /// Processing loop for the service handles recieved messages and
    /// executing actions from the message handle results
    ///
    /// `service` The service this context is processing for
    async fn process(&mut self, service: &mut S) {
        while let Some(msg) = self.rx.recv().await {
            let action = msg.handle(service, self);
            match action {
                ServiceAction::Stop => break,
                ServiceAction::Continue => continue,
                // Execute tasks that require blocking the processing
                ServiceAction::Execute(fut) => fut.await,
            }
        }
    }

    /// Stop the context directly by closing the reciever the
    /// reciever will drain any existing messages until there
    /// are none remaining
    pub fn stop(&mut self) {
        self.rx.close()
    }

    /// Creates and returns a link to the service
    pub fn link(&self) -> Link<S> {
        self.link.clone()
    }

    /// Returns a reference to the shared link used by this context
    /// for creating new links
    pub fn shared_link(&self) -> &Link<S> {
        &self.link
    }
}
