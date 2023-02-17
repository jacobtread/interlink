use futures::future::BoxFuture;

/// Trait implemented by structures that can be spawned as
/// services and used by the app
pub trait Service: Sized + Send + 'static {
    fn started(&mut self) {}
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
