use axum::extract::ws::{CloseFrame, Message as AxumMessage, WebSocket};
use futures::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TungsteniteCloseFrame;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

fn to_tungstenite(message: AxumMessage) -> TungsteniteMessage {
    match message {
        AxumMessage::Text(t) => TungsteniteMessage::Text(t.into()),
        AxumMessage::Binary(b) => TungsteniteMessage::Binary(b.into()),
        AxumMessage::Ping(p) => TungsteniteMessage::Ping(p.into()),
        AxumMessage::Pong(p) => TungsteniteMessage::Pong(p.into()),
        AxumMessage::Close(frame) => {
            TungsteniteMessage::Close(frame.map(|f| TungsteniteCloseFrame {
                code: CloseCode::from(f.code),
                reason: f.reason.to_string().into(),
            }))
        }
    }
}

fn to_axum(message: TungsteniteMessage) -> Option<AxumMessage> {
    match message {
        TungsteniteMessage::Text(t) => Some(AxumMessage::Text(t.to_string())),
        TungsteniteMessage::Binary(b) => Some(AxumMessage::Binary(b.to_vec())),
        TungsteniteMessage::Ping(p) => Some(AxumMessage::Ping(p.to_vec())),
        TungsteniteMessage::Pong(p) => Some(AxumMessage::Pong(p.to_vec())),
        TungsteniteMessage::Close(frame) => Some(AxumMessage::Close(frame.map(|f| CloseFrame {
            code: u16::from(f.code),
            reason: f.reason.to_string().into(),
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}

pub async fn bridge<S>(socket: WebSocket, upstream: tokio_tungstenite::WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut client_tx, mut client_rx) = socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let inbound = async {
        while let Some(Ok(message)) = client_rx.next().await {
            let closing = matches!(message, AxumMessage::Close(_));
            if upstream_tx.send(to_tungstenite(message)).await.is_err() || closing {
                break;
            }
        }
        let _ = upstream_tx.close().await;
    };
    let outbound = async {
        while let Some(Ok(message)) = upstream_rx.next().await {
            let Some(message) = to_axum(message) else {
                continue;
            };
            let closing = matches!(message, AxumMessage::Close(_));
            if client_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = client_tx.close().await;
    };
    tokio::select! {
        _ = inbound => {}
        _ = outbound => {}
    }
}
