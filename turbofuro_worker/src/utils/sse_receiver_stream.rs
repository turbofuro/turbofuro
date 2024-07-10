use axum::response::{sse, Sse};
use futures_util::Stream;
use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{
    mpsc::{self, Receiver},
    oneshot,
};
use turbofuro_runtime::StorageValue;

pub fn sse_handler(
    event_receiver: mpsc::Receiver<Result<sse::Event, Infallible>>,
    keep_alive: Option<sse::KeepAlive>,
    disconnect_sender: oneshot::Sender<StorageValue>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    let mut sse = Sse::new(SseReceiverStream::new(event_receiver, disconnect_sender));
    if let Some(keep_alive) = keep_alive {
        sse = sse.keep_alive(keep_alive);
    }
    sse
}

/// A wrapper around [`tokio::sync::mpsc::Receiver`] that implements [`Stream`].
///
/// [`tokio::sync::mpsc::Receiver`]: struct@tokio::sync::mpsc::Receiver
/// [`Stream`]: trait@crate::Stream
#[derive(Debug)]
pub struct SseReceiverStream<T> {
    inner: Receiver<T>,
    disconnect_sender: Option<oneshot::Sender<StorageValue>>,
}

impl<T> SseReceiverStream<T> {
    pub fn new(recv: Receiver<T>, disconnect_sender: oneshot::Sender<StorageValue>) -> Self {
        Self {
            inner: recv,
            disconnect_sender: Some(disconnect_sender),
        }
    }
}

impl<T> Drop for SseReceiverStream<T> {
    fn drop(&mut self) {
        if let Some(disconnect_sender) = self.disconnect_sender.take() {
            match disconnect_sender.send(StorageValue::Null(None)) {
                Ok(_) => {}
                Err(_) => {
                    tracing::debug!("Failed to send report disconnect signal for SSE stream. This might be normal if the actor was terminated and the stream is closed as the result of that.");
                }
            }
        }
    }
}

impl<T> Stream for SseReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_recv(cx)
    }
}
