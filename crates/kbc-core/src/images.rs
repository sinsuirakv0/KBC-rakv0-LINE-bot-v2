use std::time::Duration;

use reqwest::{Client, redirect::Policy};
use tokio::sync::Semaphore;

use crate::Result;

pub struct ImageService {
    client: Client,
    permits: Semaphore,
}

impl ImageService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(Policy::none())
                .build()?,
            permits: Semaphore::new(2),
        })
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let parsed = reqwest::Url::parse(url)?;
        if parsed.scheme() != "https"
            || !matches!(
                parsed.host_str(),
                Some("jarjarblink.github.io" | "ponosgames.com")
            )
        {
            return Err("InvalidImageHost".into());
        }
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| "ImageConcurrencyLimit")?;
        let mut response = self.client.get(parsed).send().await?.error_for_status()?;
        const MAX_BYTES: usize = 2 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|bytes| bytes > MAX_BYTES as u64)
        {
            return Err("ImageByteLimit".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("ImageByteLimit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("InvalidImageType".into());
        }
        Ok(bytes)
    }
}
