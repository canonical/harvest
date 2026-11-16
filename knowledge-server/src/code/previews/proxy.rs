use axum::{
    body::{Body, Bytes},
    extract::ws::WebSocketUpgrade,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub const PASS_COOKIE: &str = "harvest_preview";
const PRIVATE_COOKIES: [&str; 2] = [PASS_COOKIE, "token"];
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
const NOT_FORWARDED: [&str; 3] = ["host", "content-length", "origin"];
const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

pub struct Visitor<'a> {
    pub host: &'a str,
    pub scheme: &'a str,
}

pub fn forwarded_headers(incoming: &HeaderMap, visitor: &Visitor<'_>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        let lower = name.as_str();
        if HOP_BY_HOP.contains(&lower) || NOT_FORWARDED.contains(&lower) || lower.starts_with("sec-websocket-") {
            continue;
        }
        if name == header::COOKIE {
            if let Some(kept) = without_private_cookies(value) {
                headers.append(header::COOKIE, kept);
            }
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    if let Ok(host) = HeaderValue::from_str(visitor.host) {
        headers.insert(X_FORWARDED_HOST, host);
    }
    if let Ok(scheme) = HeaderValue::from_str(visitor.scheme) {
        headers.insert(X_FORWARDED_PROTO, scheme);
    }
    headers
}

fn without_private_cookies(value: &HeaderValue) -> Option<HeaderValue> {
    let kept: Vec<&str> = value
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .filter(|cookie| {
            let name = cookie.split('=').next().unwrap_or_default();
            !cookie.is_empty() && !PRIVATE_COOKIES.contains(&name)
        })
        .collect();
    if kept.is_empty() {
        return None;
    }
    HeaderValue::from_str(&kept.join("; ")).ok()
}

pub fn path_and_query(uri: &Uri) -> &str {
    uri.path_and_query().map_or("/", |p| p.as_str())
}

pub async fn forward(
    http: &reqwest::Client,
    address: &str,
    method: Method,
    uri: &Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, reqwest::Error> {
    let target = format!("http://{address}{}", path_and_query(uri));
    let upstream = http.request(method, target).headers(headers).body(body).send().await?;
    Ok(relay(upstream))
}

fn relay(upstream: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if !HOP_BY_HOP.contains(&name.as_str()) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

pub async fn tunnel(ws: WebSocketUpgrade, address: &str, uri: &Uri, incoming: &HeaderMap, visitor: &Visitor<'_>) -> Response {
    let Ok(mut request) = format!("ws://{address}{}", path_and_query(uri)).into_client_request() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    request.headers_mut().extend(forwarded_headers(incoming, visitor));
    if let Some(protocols) = incoming.get(header::SEC_WEBSOCKET_PROTOCOL) {
        request.headers_mut().insert(header::SEC_WEBSOCKET_PROTOCOL, protocols.clone());
    }
    let (upstream, response) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("the app refused the websocket: {e}")).into_response(),
    };
    let chosen = response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|p| p.to_str().ok())
        .map(String::from);
    let ws = match chosen {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.on_upgrade(move |socket| crate::websocket_bridge::bridge(socket, upstream))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visitor() -> Visitor<'static> {
        Visitor { host: "abc.preview.example.com", scheme: "https" }
    }

    #[test]
    fn private_cookies_and_hop_by_hop_headers_stay_behind() {
        let mut incoming = HeaderMap::new();
        incoming.insert(header::COOKIE, HeaderValue::from_static("harvest_preview=pass; theme=dark; token=login"));
        incoming.insert(header::HOST, HeaderValue::from_static("abc.preview.example.com"));
        incoming.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        incoming.insert(header::ORIGIN, HeaderValue::from_static("https://abc.preview.example.com"));
        incoming.insert(header::SEC_WEBSOCKET_KEY, HeaderValue::from_static("k"));
        incoming.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        let forwarded = forwarded_headers(&incoming, &visitor());
        assert_eq!(forwarded.get(header::COOKIE).unwrap(), "theme=dark");
        for gone in [header::HOST, header::CONNECTION, header::ORIGIN, header::SEC_WEBSOCKET_KEY] {
            assert!(forwarded.get(&gone).is_none(), "{gone}");
        }
        assert_eq!(forwarded.get(header::ACCEPT).unwrap(), "text/html");
    }

    #[test]
    fn the_app_learns_its_public_address() {
        let forwarded = forwarded_headers(&HeaderMap::new(), &visitor());
        assert_eq!(forwarded.get("x-forwarded-host").unwrap(), "abc.preview.example.com");
        assert_eq!(forwarded.get("x-forwarded-proto").unwrap(), "https");
    }

    #[test]
    fn a_request_with_only_private_cookies_forwards_none() {
        let mut incoming = HeaderMap::new();
        incoming.insert(header::COOKIE, HeaderValue::from_static("harvest_preview=pass"));
        assert!(forwarded_headers(&incoming, &visitor()).get(header::COOKIE).is_none());
    }

    #[test]
    fn the_full_path_and_query_are_kept() {
        let uri: Uri = "/src/main.js?t=1".parse().unwrap();
        assert_eq!(path_and_query(&uri), "/src/main.js?t=1");
        assert_eq!(path_and_query(&"http://h".parse().unwrap()), "/");
    }

    mod live {
        use std::net::SocketAddr;

        use axum::extract::ws::{Message, WebSocket};
        use axum::routing::{any, get};
        use axum::Router;
        use futures::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        use super::super::*;

        async fn serve(app: Router) -> SocketAddr {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            address
        }

        async fn echo(socket: WebSocket) {
            let (mut tx, mut rx) = socket.split();
            while let Some(Ok(Message::Text(text))) = rx.next().await {
                if tx.send(Message::Text(format!("echo {text}"))).await.is_err() {
                    break;
                }
            }
        }

        async fn app_stub() -> SocketAddr {
            serve(
                Router::new()
                    .route("/ws", get(|ws: WebSocketUpgrade| async { ws.protocols(["vite-hmr"]).on_upgrade(echo) }))
                    .fallback(any(|uri: Uri, headers: HeaderMap| async move {
                        let cookie = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()).unwrap_or("-").to_string();
                        let forwarded = headers.get("x-forwarded-host").and_then(|c| c.to_str().ok()).unwrap_or("-").to_string();
                        format!("{} cookie={cookie} host={forwarded}", path_and_query(&uri))
                    })),
            )
            .await
        }

        async fn proxy_to(app: SocketAddr) -> SocketAddr {
            let address = app.to_string();
            let handler = move |ws: Option<WebSocketUpgrade>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
                let address = address.clone();
                async move {
                    let visitor = Visitor { host: "p.preview.test", scheme: "http" };
                    match ws {
                        Some(ws) => tunnel(ws, &address, &uri, &headers, &visitor).await,
                        None => forward(&reqwest::Client::new(), &address, method, &uri, forwarded_headers(&headers, &visitor), body)
                            .await
                            .unwrap(),
                    }
                }
            };
            serve(Router::new().fallback(handler)).await
        }

        #[tokio::test]
        async fn requests_keep_their_full_path_and_lose_private_cookies() {
            let proxy = proxy_to(app_stub().await).await;
            let body = reqwest::Client::new()
                .get(format!("http://{proxy}/@vite/client?t=1"))
                .header(header::COOKIE, "harvest_preview=pass; theme=dark")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert_eq!(body, "/@vite/client?t=1 cookie=theme=dark host=p.preview.test");
        }

        #[tokio::test]
        async fn websockets_are_tunnelled_with_their_subprotocol() {
            let proxy = proxy_to(app_stub().await).await;
            let mut request = format!("ws://{proxy}/ws").into_client_request().unwrap();
            request
                .headers_mut()
                .insert(header::SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("vite-hmr"));
            let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
            assert_eq!(response.headers().get(header::SEC_WEBSOCKET_PROTOCOL).unwrap(), "vite-hmr");
            socket.send(ClientMessage::Text("hi".into())).await.unwrap();
            let reply = socket.next().await.unwrap().unwrap();
            assert_eq!(reply.into_text().unwrap().as_str(), "echo hi");
        }
    }
}
