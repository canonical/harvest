use anyhow::Result;

const OVERLOAD_RETRY_DELAY_SECS: u64 = 5;
const RATE_LIMIT_BASE_DELAY_SECS: u64 = 30;

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
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(RATE_LIMIT_BASE_DELAY_SECS * (1u64 << attempt.min(4)));
            attempt += 1;
            tracing::warn!(attempt, delay_secs = delay, provider, "rate limited — retrying");
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            continue;
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
