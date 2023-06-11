//! # Services
//!
//! Interlink uses services to represent asynchronous tasks that can hold their
//! own state and communicate between each-other using messages and handlers.
//!
//! You can derive the service trait using its derive macro or implement it
//! manually to get access to the [`Service::started`] and [`Service::stopping`]
//! functions.
//!
//! With derive macro:
//! ```
//! use interlink::prelude::*;
//!
//! #[derive(Service)]
//! struct MyService {
//!     value: String,
//! }
//! ```
//!
//! Without derive macro. Both the started and stopping functions are optional
//! ```
//! use interlink::prelude::*;
//!
//! struct MyService {
//!     value: String,
//! }
//!
//! impl Service for MyService {
//!     fn started(&mut self, ctx: &mut ServiceContext<Self>) {
//!         println!("My service is started")
//!     }
//! }
//! ```
//!
//! Then you can start your service using the [`Service::start`] function which will
//! start the service in a tokio task and return you a [`Link`] to the service. Alternatively
//! if you need access to the service context while creating your service you can use the
//! [`Service::create`] function which provides the context.
//!
//! See [`Link`] for what to do from here

use std::time::Duration;

use crate::envelope::{ServiceAction, ServiceMessage};
use crate::link::Link;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::sleep;

/// Trait implemented by structures that can be spawned as
/// services and used by the app
#[allow(unused_variables)]
pub trait Service: Sized + Send + 'static {
    /// Handler called before the service starts processing messages
    ///
    /// `ctx` The service context
    fn started(&mut self, ctx: &mut ServiceContext<Self>) {}

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

    /// Spawns this service into a new tokio task
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
    /// for creating new links. You can use this to access the service
    /// link without creating a clone of it
    pub fn shared_link(&self) -> &Link<S> {
        &self.link
    }

    /// Executes the provided `action` function on the service and service context
    /// after the provided `duration` has elapsed. This function returns a [`JoinHandle`]
    /// which you can use to stop the task by calling .abort() on the handle
    ///
    /// ```
    /// use interlink::prelude::*;
    /// use std::time::Duration;
    /// use tokio::time::sleep;
    ///
    /// struct Test {
    ///     value: u32,
    /// }
    ///
    /// impl Service for Test {
    ///     fn started(&mut self, ctx: &mut ServiceContext<Self>) {
    ///         ctx.run_later(Duration::from_secs(1), |service, _ctx| {
    ///             println!("Hello 1 second later from the service: {}", service.value);
    ///             service.value += 1;
    ///         });
    ///     }
    /// }
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///    let link = Test { value: 1 }.start();
    ///
    ///    sleep(Duration::from_secs(5)).await;
    /// }
    /// ```
    pub fn run_later<F>(&self, duration: Duration, action: F) -> JoinHandle<()>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) + Send + 'static,
    {
        let link = self.link();
        tokio::spawn(async move {
            sleep(duration).await;
            let _ = link.do_exec(action);
        })
    }

    /// Executes the provided `action` function on the service and service context
    /// every time the `duration` is elapsed. This function returns a [`JoinHandle`]
    /// which you can use to stop the task by calling .abort() on the handle
    ///
    /// ```
    /// use interlink::prelude::*;
    /// use std::time::Duration;
    /// use tokio::time::sleep;
    ///
    /// struct Test {
    ///     value: u32,
    /// }
    ///
    /// impl Service for Test {
    ///     fn started(&mut self, ctx: &mut ServiceContext<Self>) {
    ///         ctx.run_interval(Duration::from_secs(1), |service, _ctx| {
    ///             println!(
    ///                 "Hello at 1 second interval from the service: {}",
    ///                 service.value
    ///              );
    ///             service.value += 1;
    ///         });
    ///     }
    /// }
    ///
    /// #[tokio::test]
    /// async fn test() {
    ///    let link = Test { value: 1 }.start();
    ///
    ///    sleep(Duration::from_secs(15)).await;
    /// }
    /// ```
    pub fn run_interval<F>(&self, duration: Duration, action: F) -> JoinHandle<()>
    where
        F: FnOnce(&mut S, &mut ServiceContext<S>) + Clone + Send + 'static,
    {
        let link = self.link();
        tokio::spawn(async move {
            loop {
                sleep(duration).await;
                if link.do_exec(action.clone()).is_err() {
                    break;
                }
            }
        })
    }
}
