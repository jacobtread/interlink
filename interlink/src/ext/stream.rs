//! # Stream Support
//!
//! This module provides support for attaching futures [`Stream`]'s to services
//! in order to handle a stream of incoming messages.
//!
//! Use [`ServiceContext::attach_stream`] to attach a stream to your service and
//! implement the [`StreamHandler`] trait on that service to handle a stream of
//! messages

use crate::{
    envelope::StreamEnvelope,
    link::Link,
    msg::StreamHandler,
    service::{Service, ServiceContext},
};
use futures_core::stream::Stream;
use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

impl<S> ServiceContext<S>
where
    S: Service,
{
    /// Attaches a streaming reciever to the service context
    ///
    /// implement the StreamHandler trait on your service
    /// with the item as the Item type of the provided stream
    /// in order to handle accepting items from the stream
    ///
    /// `stream` The stream to accept from
    /// `stop`   Whether to stop the main service when this stream service ends
    pub fn attach_stream<St>(&self, stream: St, stop: bool)
    where
        S: StreamHandler<St::Item>,
        St: Stream + Send + Unpin + 'static,
        St::Item: Send + 'static,
    {
        StreamService::start(stream, self.link(), stop)
    }
}
/// Service for reading items from a stream and sending the
/// items to the StreamHandler on the provided service can
/// optionally stop the provided service when there are
/// no more items.
struct StreamService<S, St> {
    /// The stream to consume
    stream: St,
    /// Link to the associated service
    link: Link<S>,
    /// Whether to stop the associated service when this stream ends
    stop: bool,
}

impl<S, St> StreamService<S, St>
where
    S: Service + StreamHandler<St::Item>,
    St: Stream + Send + Unpin + 'static,
    St::Item: Send + 'static,
{
    /// Starts a new stream service
    ///
    /// `stream` The stream to accept items from
    /// `link`   Link to the service that will handle the items
    /// `stop`   If true the linked service will be stopped when there are no more items
    pub(crate) fn start(stream: St, link: Link<S>, stop: bool) {
        let service = StreamService { stream, link, stop };
        tokio::spawn(service);
    }

    /// Pins the underlying stream and polls for the next
    /// item in the stream
    ///
    /// `cx` The polling context
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<St::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

impl<S, St> Future for StreamService<S, St>
where
    S: Service + StreamHandler<St::Item>,
    St: Stream + Send + Unpin + 'static,
    St::Item: Send + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        while let Some(item) = ready!(this.poll_next(cx)) {
            if this.link.tx(StreamEnvelope::new(item)).is_err() {
                // Linked service has ended stop processing
                // early return to skip calling stop
                return Poll::Ready(());
            }
        }

        if this.stop {
            // Stop the linked service because there are no more items
            this.link.stop();
        }

        Poll::Ready(())
    }
}
