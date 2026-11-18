use std::pin::Pin;

use axum::extract::ws::{Message as BrowserMessage, WebSocket};
use futures::future::ready;
use futures::stream::BoxStream;
use futures::{Sink, SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as LxdMessage;

use super::bridge::{BrowserEnd, ShellEnd};
use super::protocol::{self, Frame, FromBrowser, ToBrowser};
use crate::lxd::console::Console;

pub type BoxSink<T> = Pin<Box<dyn Sink<T, Error = anyhow::Error> + Send>>;
pub type Browser = BrowserEnd<BoxStream<'static, FromBrowser>, BoxSink<ToBrowser>>;
pub type Shell = ShellEnd<BoxStream<'static, Vec<u8>>, BoxSink<Vec<u8>>, BoxSink<String>>;

pub fn browser(socket: WebSocket) -> Browser {
    let (outgoing, incoming) = socket.split();
    let incoming = incoming
        .take_while(|message| ready(matches!(message, Ok(message) if !matches!(message, BrowserMessage::Close(_)))))
        .filter_map(|message| ready(message.ok().and_then(frame_from_browser).and_then(protocol::from_browser)))
        .boxed();
    let outgoing = outgoing
        .sink_map_err(anyhow::Error::from)
        .with(|message: ToBrowser| ready(Ok::<_, anyhow::Error>(browser_message(protocol::to_browser(message)))));
    BrowserEnd { incoming, outgoing: Box::pin(outgoing) }
}

pub fn shell(console: Console) -> Shell {
    let (input, output) = console.data.split();
    let (control, _) = console.control.split();
    let output = output
        .take_while(|message| ready(matches!(message, Ok(message) if !message.is_close())))
        .filter_map(|message| ready(message.ok().filter(LxdMessage::is_binary).map(|message| message.into_data().to_vec())))
        .boxed();
    let input = input
        .sink_map_err(anyhow::Error::from)
        .with(|bytes: Vec<u8>| ready(Ok::<_, anyhow::Error>(LxdMessage::binary(bytes))));
    let control = control
        .sink_map_err(anyhow::Error::from)
        .with(|text: String| ready(Ok::<_, anyhow::Error>(LxdMessage::text(text))));
    ShellEnd { output, input: Box::pin(input), control: Box::pin(control) }
}

fn frame_from_browser(message: BrowserMessage) -> Option<Frame> {
    match message {
        BrowserMessage::Binary(bytes) => Some(Frame::Binary(bytes)),
        BrowserMessage::Text(text) => Some(Frame::Text(text)),
        BrowserMessage::Ping(_) | BrowserMessage::Pong(_) | BrowserMessage::Close(_) => None,
    }
}

fn browser_message(frame: Frame) -> BrowserMessage {
    match frame {
        Frame::Binary(bytes) => BrowserMessage::Binary(bytes),
        Frame::Text(text) => BrowserMessage::Text(text),
        Frame::Ping => BrowserMessage::Ping(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_messages_become_frames_and_back() {
        assert_eq!(frame_from_browser(BrowserMessage::Binary(b"x".to_vec())), Some(Frame::Binary(b"x".to_vec())));
        assert_eq!(frame_from_browser(BrowserMessage::Text("{}".into())), Some(Frame::Text("{}".into())));
        assert_eq!(frame_from_browser(BrowserMessage::Pong(Vec::new())), None);
        assert!(matches!(browser_message(Frame::Ping), BrowserMessage::Ping(_)));
    }
}
