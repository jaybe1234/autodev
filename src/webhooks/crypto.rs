use axum::http::HeaderMap;
use eyre::WrapErr;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_webhook_signature(
    headers: &HeaderMap,
    body: &[u8],
    secret: &str,
) -> Result<(), AppError> {
    let signature = headers
        .get("x-hub-signature-256")
        .or_else(|| headers.get("x-hub-signature"))
        .ok_or(AppError::WebhookVerification)?
        .to_str()
        .with_context(|| "reading webhook signature header")
        .map_err(AppError::from)?;

    let signature = signature
        .strip_prefix("sha256=")
        .unwrap_or(signature);

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body);

    let result = mac.finalize();
    let computed = hex::encode(result.into_bytes());

    if !timing_safe_eq(&computed, signature) {
        return Err(AppError::WebhookVerification);
    }

    Ok(())
}

pub fn timing_safe_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0, |acc, (x, y)| acc | (x ^ y))
        == 0
}
