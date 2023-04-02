//! # Sink Support
//!
//! This module provides support for attaching futures [`Sink`]'s to your
//! services to handle sending messages easily
//!
//! Use [`ServiceContext::attach_sink`] to attach a sink to your service

use crate::{
    envelope::ErrorEnvelope,
    link::{Link, LinkError},
    msg::ErrorHandler,
    service::{Service, ServiceContext},
};
use futures_sink::Sink;
use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::mpsc;

impl<S: Service> ServiceContext<S> {
    /// Attaches a sink to this service and provides a link to the
    /// service so that it can be used to write messages
    ///
    /// `sink` The sink to attach
    pub fn attach_sink<Si, I>(&self, sink: Si) -> SinkLink<I>
    where
        S: ErrorHandler<Si::Error>,
        Si: Sink<I> + Send + Unpin + 'static,
        Si::Error: Send + 'static,
        I: Send + 'static,
    {
        SinkService::start(sink, self.link())
    }
}

/// Dedicated link type for sinks.
///
/// This is cheaply clonable so you can clone
/// it to use it in multiple places.
pub struct SinkLink<I>(mpsc::UnboundedSender<SinkMessage<I>>);

impl<I> Clone for SinkLink<I> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Messages used to communicate internally with the sink
enum SinkMessage<I> {
    /// Feed an item into the sink optionally flushing
    Feed { item: I, flush: bool },
    /// Flush all the items in the sink
    Flush,
    /// Tells the sink to stop processing values
    Stop,
}

/// Additional logic for sink links
impl<I> SinkLink<I>
where
    I: Send,
{
    /// Sends a new item to the sink. This item will be written
    /// to the underlying target as soon as the previous messages
    /// are written.
    ///
    /// This does not required calling flush.
    ///
    /// Any previous calls to feed will be flushed with this message
    ///
    /// `item` The item to write
    pub fn sink(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Feed { item, flush: true })
            .map_err(|_| LinkError::Send)
    }

    /// Feeds a new item to the sink this item will not be written
    /// until flush is called.
    ///
    /// This requires flush be called at some point to send
    ///
    /// `item` The item to feed
    pub fn feed(&self, item: I) -> Result<(), LinkError> {
        self.0
            .send(SinkMessage::Feed { item, flush: false })
            .map_err(|_| LinkError::Send)
    }

    /// Requests that the Sink service flush any messages that have
    /// been fed to the sink
    pub fn flush(&self) -> Result<(), LinkError> {
        self.0.send(SinkMessage::Flush).map_err(|_| LinkError::Send)
    }

    /// Requests that the sink shutdown and stop accepting new
    /// messages. This will drop the underlying write target
    pub fn stop(&self) -> Result<(), LinkError> {
        self.0.send(SinkMessage::Stop).map_err(|_| LinkError::Send)
    }
}

/// Service for handling a Sink and its writing this is a
/// lightweight service which has its own link type and doesn't
/// implement normal service logic to be more lightweight
struct SinkService<S, Si, I> {
    /// The underlying sink to write to
    sink: Si,
    /// The link to the attached service
    link: Link<S>,
    /// Receiver for sink messages
    rx: mpsc::UnboundedReceiver<SinkMessage<I>>,
    /// The current sink service action
    action: Option<FutState<I>>,
}

impl<S, Si, I> SinkService<S, Si, I>
where
    S: Service + ErrorHandler<Si::Error>,
    Si: Sink<I> + Send + Unpin + 'static,
    Si::Error: Send + 'static,
    I: Send + 'static,
{
    /// Starts a new sink service. You should attach this
    ///
    /// `sink` The sink to send and feed the items into
    /// `link` Link to the service that will handle the items
    pub(crate) fn start(sink: Si, link: Link<S>) -> SinkLink<I> {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink_link = SinkLink(tx);
        let service = SinkService {
            sink,
            link,
            rx,
            action: None,
        };
        tokio::spawn(service);
        sink_link
    }

    /// Handlings polling the provided action on the provided sink
    ///
    /// `sink`   The sink to poll against
    /// `state`  The state to poll
    /// `cx`     The polling context
    fn poll_sink(
        mut sink: Pin<&mut Si>,
        state: &mut FutState<I>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Si::Error>> {
        let flush = match state {
            FutState::Feed { item, flush } => {
                if item.is_some() {
                    // Poll the sink until its ready
                    ready!(sink.as_mut().poll_ready(cx))?;

                    // Send the item to the sink
                    let item = item.take().expect("polled feed after completion");
                    sink.as_mut().start_send(item)?;
                }
                *flush
            }
            FutState::Flush => true,
        };

        // Flush the sink if required
        if flush {
            sink.poll_flush(cx)
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

/// State for what the service is currently doing
enum FutState<I> {
    /// Feeding into the sink
    Feed {
        /// The item to feed
        item: Option<I>,
        /// Whether to flush after feeding the item
        flush: bool,
    },
    /// Flushing the sink
    Flush,
}

impl<I> Unpin for FutState<I> {}

impl<S, Si, I> Future for SinkService<S, Si, I>
where
    S: Service + ErrorHandler<Si::Error>,
    Si: Sink<I> + Send + Unpin + 'static,
    Si::Error: Send + 'static,
    I: Send + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            this.action = if let Some(action) = &mut this.action {
                let sink = Pin::new(&mut this.sink);
                if let Err(err) = ready!(Self::poll_sink(sink, action, cx)) {
                    if this.link.tx(ErrorEnvelope::new(err)).is_err() {
                        // If the error message couldn't be sent the service is stopped
                        break;
                    }
                }
                None
            } else {
                let msg = match ready!(this.rx.poll_recv(cx)) {
                    Some(value) => value,
                    // Nothing left to recv
                    None => break,
                };

                let state = match msg {
                    SinkMessage::Feed { item, flush } => FutState::Feed {
                        item: Some(item),
                        flush,
                    },
                    SinkMessage::Flush => FutState::Flush,
                    // Nothing left to recv stop processing
                    SinkMessage::Stop => break,
                };
                Some(state)
            }
        }

        Poll::Ready(())
    }
}
