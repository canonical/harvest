pub fn dial_back_ws_url(server_url: &str, path: &str) -> String {
    let ws_base = if let Some(rest) = server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        server_url.to_string()
    };
    format!("{}{path}", ws_base.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_https_to_wss() {
        let url = dial_back_ws_url("https://harvest.example.com", "/agent/console/sess-1");
        assert_eq!(url, "wss://harvest.example.com/agent/console/sess-1");
    }

    #[test]
    fn converts_http_to_ws() {
        let url = dial_back_ws_url("http://localhost:8080", "/agent/tunnel/sess-1");
        assert_eq!(url, "ws://localhost:8080/agent/tunnel/sess-1");
    }

    #[test]
    fn strips_trailing_slash() {
        let url = dial_back_ws_url("https://harvest.example.com/", "/agent/console/sess-1");
        assert_eq!(url, "wss://harvest.example.com/agent/console/sess-1");
    }

    #[test]
    fn leaves_unrecognised_scheme_untouched() {
        let url = dial_back_ws_url("ws://already-ws.example.com", "/agent/tunnel/sess-1");
        assert_eq!(url, "ws://already-ws.example.com/agent/tunnel/sess-1");
    }
}
