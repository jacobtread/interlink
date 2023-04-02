//! Extensions providing supports for attaching streams and sinks to
//! your services.

#[cfg(feature = "sinks")]
pub mod sink;
#[cfg(feature = "streams")]
pub mod stream;
