use crate::{ctx::ServiceContext, link::Link};

/// Trait implemented by structures that can be spawned as
/// services and used by the app
pub trait Service: Sized + Send + 'static {
    /// Handler called before the service starts processing messages
    ///
    /// `ctx` The service context
    fn started(&mut self, ctx: &mut ServiceContext<Self>) {}

    /// Start an already created service and provides a link for
    /// communicating with the service
    ///
    /// ```
    ///
    /// struct MyService;
    ///
    /// // Implement the service trait
    /// impl Service for MyService {}
    ///
    /// // Create the service
    /// let service: MyService = MyService {};
    /// // Start the service and obtain a link to it
    /// let addr: Link<MyService> = service.start();
    ///
    ///
    /// ```
    fn start(self) -> Link<Self> {
        let ctx = ServiceContext::new();
        let link = ctx.link();
        self.spawn(ctx);
        link
    }

    /// Alternative way of creating a service where the service may
    /// rely on the context an example of this is an associated service
    /// which requires a link to the service but is also stored on the
    /// service struct
    ///
    /// ```
    /// struct First {
    ///     /// Link to spawned service
    ///     second: Link<Second>,
    /// }
    ///
    /// /// Some other service which requires a link to our service
    /// struct Second {
    ///     /// Link to the service that owns this service
    ///     owner: Link<First>
    /// }
    ///
    /// // Provide a closure which takes in the ctx    
    /// First::create(|ctx| {
    ///     
    ///     // Create second which references the context
    ///     let second: Second = Second {
    ///         owner: ctx.link()
    ///     }
    ///     .start();
    ///     
    ///     // Can now use the spawned value
    ///     First { second }    
    /// })
    ///
    fn create<F>(action: F) -> Link<Self>
    where
        F: FnOnce(&mut ServiceContext<Self>) -> Self,
    {
        let mut ctx = ServiceContext::new();
        let this = action(&mut ctx);
        let link = ctx.link();
        this.spawn(ctx);
        link
    }

    /// Spawns this servuce  into a new tokio task
    /// where it will then begin processing messages
    ///
    /// This should not be called manually use
    /// `start` or `create` instead
    ///
    /// `ctx` The service context
    fn spawn(mut self, mut ctx: ServiceContext<Self>) {
        tokio::spawn(async move {
            self.started(&mut ctx);

            ctx.process(&mut self).await;

            self.stopping();
        });
    }

    /// Handler logic called when the service is stopping
    fn stopping(&mut self) {}
}
