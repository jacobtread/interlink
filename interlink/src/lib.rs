//! # Interlink
//!
//! Interlink is an async framework
//!
//! # Starting a service
//!
//! In order to get a link to a service and for the service to run you will first need to start the service
//!
//! ```
//! use interlink::prelude::*;
//!
//! /// Define your backing structure for the service you can use
//! /// the service derive macro here or implement the trait to
//! /// get access to the `started` and `stopping` hooks
//! #[derive(Service)]
//! struct Example;
//!
//! // You must be within the tokio runtime to use interlink
//! #[tokio::main]
//! async fn main() {
//!     // Create the service
//!     let service = Example {};
//!     // Start the service to get a link to the service
//!     let link = service.start();
//! }
//!```
//!
//! # Sending a message to a service
//!
//! To communicate with services and between services you use messages below is an example of how to create and send messages.
//!
//!```
//! use interlink::prelude::*;
//!
//! // Define your backing structure for the service
//! #[derive(Service)]
//! struct Example;
//!
//! // The message struct with a string response type
//! #[derive(Message)]
//! #[msg(rtype = "String")]
//! struct TextMessage {
//!     value: String,
//! }
//!
//! /// Implement a handler for the message type
//! impl Handler<TextMessage> for Example {
//!
//!     /// Basic response type which just responds with the value
//!     type Response = Mr<TextMessage>;
//!
//!     fn handle(
//!         &mut self,
//!         msg: TextMessage,
//!         ctx: &mut ServiceContext<Self>
//!     ) -> Self::Response {
//!         println!("Got message: {}", &msg.value);
//!         Mr(msg.value)
//!     }
//! }
//!
//! // You must be within the tokio runtime to use interlink
//! #[tokio::main]
//! async fn main() {
//!     // Create the service
//!     let service = Example {};
//!     // Start the service to get a link to the service
//!     let link = service.start();
//!
//!     // Send the text message to the service and await the response
//!     let res: String = link.send(TextMessage {
//!             value: "Example".to_string(),
//!         })
//!         .await
//!         .unwrap();
//!
//!     assert_eq!(&res, "Example");
//!
//!     // You can also send without waiting for a response
//!     link.do_send(TextMessage {
//!             value: "Example".to_string(),
//!         })
//!         .unwrap();
//!
//! }
//! ```

mod envelope;
pub mod ext;
pub mod link;
pub mod msg;
pub mod prelude;
pub mod service;
