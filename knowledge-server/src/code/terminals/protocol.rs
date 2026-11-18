use serde::Deserialize;
use serde_json::json;

use super::model::TerminalSize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Binary(Vec<u8>),
    Text(String),
    Ping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromBrowser {
    Input(Vec<u8>),
    Resize(TerminalSize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToBrowser {
    Output(Vec<u8>),
    Exit(Option<i64>),
    Reconnect,
    Keepalive,
}

#[derive(Deserialize)]
struct Control {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
}

pub fn from_browser(frame: Frame) -> Option<FromBrowser> {
    match frame {
        Frame::Binary(bytes) => Some(FromBrowser::Input(bytes)),
        Frame::Text(text) => {
            let control: Control = serde_json::from_str(&text).ok()?;
            (control.kind == "resize")
                .then(|| TerminalSize::new(control.cols, control.rows))
                .flatten()
                .map(FromBrowser::Resize)
        }
        Frame::Ping => None,
    }
}

pub fn to_browser(message: ToBrowser) -> Frame {
    match message {
        ToBrowser::Output(bytes) => Frame::Binary(bytes),
        ToBrowser::Exit(code) => Frame::Text(json!({ "type": "exit", "code": code }).to_string()),
        ToBrowser::Reconnect => Frame::Text(json!({ "type": "reconnect" }).to_string()),
        ToBrowser::Keepalive => Frame::Ping,
    }
}

pub fn lxd_resize(size: TerminalSize) -> String {
    json!({
        "command": "window-resize",
        "args": { "width": size.cols.to_string(), "height": size.rows.to_string() },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrokes_arrive_as_binary_input() {
        assert_eq!(from_browser(Frame::Binary(b"ls\r".to_vec())), Some(FromBrowser::Input(b"ls\r".to_vec())));
    }

    #[test]
    fn a_resize_carries_a_valid_size() {
        let resize = Frame::Text(r#"{"type":"resize","cols":132,"rows":43}"#.into());
        assert_eq!(from_browser(resize), Some(FromBrowser::Resize(TerminalSize { cols: 132, rows: 43 })));
    }

    #[test]
    fn unknown_or_invalid_control_messages_are_ignored() {
        for text in [r#"{"type":"resize","cols":0,"rows":10}"#, r#"{"type":"paste"}"#, "not json"] {
            assert_eq!(from_browser(Frame::Text(text.into())), None, "{text}");
        }
        assert_eq!(from_browser(Frame::Ping), None);
    }

    #[test]
    fn output_is_binary_and_events_are_json() {
        assert_eq!(to_browser(ToBrowser::Output(b"hi".to_vec())), Frame::Binary(b"hi".to_vec()));
        assert_eq!(to_browser(ToBrowser::Exit(Some(1))), Frame::Text(r#"{"code":1,"type":"exit"}"#.into()));
        assert_eq!(to_browser(ToBrowser::Reconnect), Frame::Text(r#"{"type":"reconnect"}"#.into()));
        assert_eq!(to_browser(ToBrowser::Keepalive), Frame::Ping);
    }

    #[test]
    fn lxd_receives_window_sizes_as_strings() {
        assert_eq!(
            lxd_resize(TerminalSize { cols: 100, rows: 30 }),
            r#"{"args":{"height":"30","width":"100"},"command":"window-resize"}"#
        );
    }
}
