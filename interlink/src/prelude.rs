/// Export for sink usage
pub use crate::ext::sink::SinkLink;

/// Message and error action exports
pub use crate::msg::{BoxFuture, ErrorAction, Message};

/// Handler exports
pub use crate::msg::{ErrorHandler, Handler, StreamHandler};

/// Link exports
pub use crate::link::{Link, LinkError, LinkResult, MessageLink};

/// Service exports
pub use crate::service::{Service, ServiceContext};

/// Message type exports
pub use crate::msg::{Fr, Mr, Sfr};

/// Re-exports for interlink derive implementation
pub use interlink_derive::{Message, Service};
