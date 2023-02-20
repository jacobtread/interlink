/// Export for sink usage
pub use crate::ctx::sink::SinkLink;

/// Message and error action exports
pub use crate::msg::{ErrorAction, Message};

/// Handler exports
pub use crate::msg::{ErrorHandler, Handler, StreamHandler};

/// Link exports
pub use crate::link::{Link, LinkError, LinkResult, MessageLink};

/// Service exports
pub use crate::ctx::ServiceContext;
pub use crate::service::Service;

/// Message type exports
pub use crate::msg::{Fr, Mr, Sfr};

/// Re-exports for interlink derive implementation
pub use interlink_derive::{Message, Service};
