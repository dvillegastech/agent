use std::time::Duration;

use colored::Colorize;
use tokio::time::sleep;

use crate::error::{AgentError, Result};

/// Maximum number of retry attempts.
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (in milliseconds).
const BASE_DELAY_MS: u64 = 1000;

/// Retry a fallible async operation with exponential backoff.
/// Only retries on transient errors (network, 429, 5xx).
pub async fn with_retry<F, Fut, T>(operation_name: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if !is_retryable(&e) || attempt == MAX_RETRIES {
                    return Err(e);
                }

                let delay = Duration::from_millis(BASE_DELAY_MS * 2u64.pow(attempt));
                eprintln!(
                    "  {} {} (attempt {}/{}, retrying in {:?})",
                    "[retry]".yellow(),
                    operation_name,
                    attempt + 1,
                    MAX_RETRIES + 1,
                    delay
                );

                last_error = Some(e);
                sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AgentError::Provider("Max retries exceeded".into())))
}

/// Determine if an error is retryable (transient).
fn is_retryable(error: &AgentError) -> bool {
    match error {
        AgentError::Http(e) => {
            // Network errors are retryable
            if e.is_connect() || e.is_timeout() {
                return true;
            }
            // Rate limits (429) and server errors (5xx) are retryable
            if let Some(status) = e.status() {
                return status.as_u16() == 429 || status.is_server_error();
            }
            false
        }
        AgentError::Provider(msg) => {
            // Rate limit or server error messages
            msg.contains("429")
                || msg.contains("rate")
                || msg.contains("overloaded")
                || msg.contains("500")
                || msg.contains("502")
                || msg.contains("503")
        }
        _ => false,
    }
}
