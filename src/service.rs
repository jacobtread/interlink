use futures::future::BoxFuture;

use crate::{ctx::ServiceContext, link::Link};

/// Trait implemented by structures that can be spawned as
/// services and used by the app
pub trait Service: Sized + Send + 'static {
    fn started(&mut self, ctx: &mut ServiceContext<Self>) {}

    fn start(self) -> Link<Self> {
        let ctx = ServiceContext::new();
        let link = ctx.link();
        self.spawn(ctx);
        link
    }

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

/// Actions that can be executed by the service processor
pub enum ServiceAction<'a> {
    /// Tell service to shutdown
    Stop,
    /// Continue handling the next message
    Continue,
    /// Ask the service to execute a future on the service
    Execute(BoxFuture<'a, ()>),
}
