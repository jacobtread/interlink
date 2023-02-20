//! Sub modules of this module are extensions upon the ServiceContext
//! that add additional features to the Context such as Stream handling

#[cfg(feature = "sinks")]
pub mod sink;
#[cfg(feature = "streams")]
pub mod stream;
