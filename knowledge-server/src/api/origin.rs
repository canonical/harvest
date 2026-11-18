use axum::http::{header, HeaderMap};

pub fn is_same_origin(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| origin.trim_end_matches('/').eq_ignore_ascii_case(expected.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(origin: Option<&'static str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_static(origin));
        }
        headers
    }

    #[test]
    fn only_the_configured_origin_is_accepted() {
        assert!(is_same_origin(&headers(Some("http://10.164.219.10")), "http://10.164.219.10"));
        assert!(is_same_origin(&headers(Some("https://Harvest.Example.com")), "https://harvest.example.com/"));
        assert!(!is_same_origin(&headers(Some("https://evil.example")), "https://harvest.example.com"));
        assert!(!is_same_origin(&headers(Some("http://harvest.example.com")), "https://harvest.example.com"));
        assert!(!is_same_origin(&headers(None), "https://harvest.example.com"));
    }
}
