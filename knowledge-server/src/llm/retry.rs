use anyhow::Result;

const OVERLOAD_RETRY_DELAY_SECS: u64 = 5;

pub(super) async fn send_with_retry<F, Fut>(
    max_retries: u32,
    overload_codes: &[u16],
    provider: &str,
    make_request: F,
) -> Result<reqwest::Response>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let mut attempt = 0u32;
    loop {
        let response = match make_request().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() && attempt < max_retries => {
                attempt += 1;
                let delay = 2u64 * (1u64 << attempt.min(4));
                tracing::warn!(attempt, delay_secs = delay, provider, "request timed out — retrying");
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        let status = response.status();
        let status_code = status.as_u16();

        if status_code == 429 && attempt < max_retries {
            if let Some(delay) = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
            {
                attempt += 1;
                tracing::warn!(attempt, delay_secs = delay, provider, "rate limited — retrying");
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                continue;
            }
            tracing::warn!(provider, "rate limited with no Retry-After — propagating without retry");
        }

        if overload_codes.contains(&status_code) && attempt < max_retries {
            attempt += 1;
            tracing::warn!(
                attempt,
                status = %status,
                provider,
                "overloaded — retrying in {OVERLOAD_RETRY_DELAY_SECS}s"
            );
            tokio::time::sleep(std::time::Duration::from_secs(OVERLOAD_RETRY_DELAY_SECS)).await;
            continue;
        }

        return Ok(response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

    async fn get(url: String) -> Result<reqwest::Response, reqwest::Error> {
        reqwest::get(&url).await
    }

    #[tokio::test]
    async fn success_returns_immediately() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/ok");
            then.status(200);
        });
        let r = send_with_retry(3, &[], "test", || get(server.url("/ok"))).await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn overload_retries_up_to_max_and_returns_last_response() {
        let server = MockServer::start();
        let calls = Arc::new(AtomicU32::new(0));
        server.mock(|when, then| {
            when.method("GET").path("/overload");
            then.status(503);
        });
        let calls2 = Arc::clone(&calls);
        let r = send_with_retry(2, &[503], "test", || {
            calls2.fetch_add(1, Ordering::SeqCst);
            get(server.url("/overload"))
        }).await.unwrap();
        assert_eq!(r.status().as_u16(), 503);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "1 original + 2 retries");
    }

    #[tokio::test]
    async fn rate_limit_without_retry_after_is_not_retried() {
        let server = MockServer::start();
        let calls = Arc::new(AtomicU32::new(0));
        server.mock(|when, then| {
            when.method("GET").path("/quota");
            then.status(429);
        });
        let calls2 = Arc::clone(&calls);
        let r = send_with_retry(3, &[], "test", || {
            calls2.fetch_add(1, Ordering::SeqCst);
            get(server.url("/quota"))
        }).await.unwrap();
        assert_eq!(r.status().as_u16(), 429);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "must not retry when no Retry-After");
    }

    #[tokio::test]
    async fn rate_limit_with_retry_after_retries() {
        let server = MockServer::start();
        let calls = Arc::new(AtomicU32::new(0));
        server.mock(|when, then| {
            when.method("GET").path("/transient");
            then.status(429).header("retry-after", "1");
        });
        let calls2 = Arc::clone(&calls);
        let r = send_with_retry(1, &[], "test", || {
            calls2.fetch_add(1, Ordering::SeqCst);
            get(server.url("/transient"))
        }).await.unwrap();
        assert_eq!(r.status().as_u16(), 429);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "must retry once when Retry-After present");
    }
}
