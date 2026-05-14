use std::any::type_name;

use eyre::{Result, WrapErr};

pub fn parse_body<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result: Result<T, _> = serde_path_to_error::deserialize(&mut deserializer);
    if let Err(ref e) = result {
        tracing::error!(path = %e.path(), error = %e, "serde deserialize failed");
    }
    result.with_context(|| {
        let preview = String::from_utf8_lossy(bytes);
        let truncated = if preview.len() > 500 {
            &preview[..500]
        } else {
            &preview
        };
        format!(
            "failed to parse {} (payload: {})",
            type_name::<T>(),
            truncated
        )
    })
}
