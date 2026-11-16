use axum::http::{header, HeaderMap, HeaderValue};

pub fn origin_of(url: &str) -> String {
    let url = url.trim();
    let after_scheme = url.find("://").map_or(0, |i| i + 3);
    let end = url[after_scheme..].find('/').map_or(url.len(), |i| after_scheme + i);
    url[..end].to_string()
}

pub fn allow_framing_by(headers: &mut HeaderMap, ancestor: &str) {
    headers.remove(header::X_FRAME_OPTIONS);
    let directive = format!("frame-ancestors {ancestor}");
    let policies: Vec<String> = headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|policy| with_frame_ancestors(policy, &directive))
        .collect();
    headers.remove(header::CONTENT_SECURITY_POLICY);
    let policies = if policies.is_empty() { vec![directive] } else { policies };
    for policy in policies {
        if let Ok(value) = HeaderValue::from_str(&policy) {
            headers.append(header::CONTENT_SECURITY_POLICY, value);
        }
    }
}

fn with_frame_ancestors(policy: &str, directive: &str) -> String {
    let mut parts: Vec<&str> = policy
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.to_ascii_lowercase().starts_with("frame-ancestors"))
        .collect();
    parts.push(directive);
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARVEST: &str = "http://10.0.0.5";

    #[test]
    fn the_origin_keeps_scheme_host_and_port_only() {
        assert_eq!(origin_of("http://10.0.0.5"), "http://10.0.0.5");
        assert_eq!(origin_of("https://harvest.example.com:8443/#/chat"), "https://harvest.example.com:8443");
    }

    #[test]
    fn frame_blocking_headers_are_replaced_by_harvest_only() {
        let mut headers = HeaderMap::new();
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
        );
        allow_framing_by(&mut headers, HARVEST);
        assert!(headers.get(header::X_FRAME_OPTIONS).is_none());
        assert_eq!(
            headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "default-src 'self'; frame-ancestors http://10.0.0.5"
        );
    }

    #[test]
    fn apps_without_a_policy_still_only_allow_harvest() {
        let mut headers = HeaderMap::new();
        allow_framing_by(&mut headers, HARVEST);
        assert_eq!(headers.get(header::CONTENT_SECURITY_POLICY).unwrap(), "frame-ancestors http://10.0.0.5");
    }
}
