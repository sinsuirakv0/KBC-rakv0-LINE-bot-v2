use reqwest::{Client, Url, redirect::Policy};
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

pub const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct AssetService {
    base: String,
    client: Client,
    permits: Arc<Semaphore>,
}

#[derive(Debug)]
pub struct AssetError(String);
impl AssetError {
    pub fn new(label: &str, detail: impl fmt::Display) -> Self {
        Self(format!("{label}: {detail}"))
    }
}
impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssetError {}

impl AssetService {
    pub fn new(commit: &str) -> crate::Result<Self> {
        if commit != "main"
            && (commit.len() != 40 || !commit.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("InvalidAssetCommit".into());
        }
        Ok(Self {
            base: format!(
                "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{commit}/jp/sitedata"
            ),
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(Policy::none())
                .build()?,
            permits: Arc::new(Semaphore::new(2)),
        })
    }
    pub fn at_revision(&self, revision: &str) -> crate::Result<Self> {
        if revision != "main"
            && (revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("InvalidAssetCommit".into());
        }
        // Clientと全体2枠を共有し、取得元だけを切り替える。存在結果は保持しない。
        Ok(Self {
            base: format!(
                "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{revision}/jp/sitedata"
            ),
            client: self.client.clone(),
            permits: Arc::clone(&self.permits),
        })
    }
    fn url(&self, path: &str) -> Result<Url, AssetError> {
        if path.starts_with('/')
            || path.contains(['\\', '?', '#', '%'])
            || path.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            })
        {
            return Err(AssetError::new("asset", "invalid path"));
        }
        Url::parse(&format!("{}/{path}", self.base)).map_err(|e| AssetError::new("asset", e))
    }
    pub async fn exists(&self, path: &str) -> Result<bool, AssetError> {
        let url = self.url(path)?;
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| AssetError::new("permit", e))?;
        let response = self
            .client
            .head(url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(|e| AssetError::new("HEAD", e))?;
        let exists = response.status() != reqwest::StatusCode::NOT_FOUND;
        if exists {
            response
                .error_for_status()
                .map_err(|e| AssetError::new("HEAD", e))?;
        }
        Ok(exists)
    }
    pub async fn bytes(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        self.download(self.url(path)?).await
    }
    pub async fn download(&self, url: Url) -> Result<Vec<u8>, AssetError> {
        if url.scheme() != "https"
            || url.host_str() != Some("raw.githubusercontent.com")
            || !url.as_str().starts_with(&format!("{}/", self.base))
        {
            return Err(AssetError::new("asset", "invalid host/path"));
        }
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| AssetError::new("permit", e))?;
        let response = self
            .client
            .get(url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| AssetError::new("GET", e))?;
        read_body(response).await
    }
    // 素材・ストア監視でClient、HTTP上限2枠、15秒timeout、4MiB上限を共有する。
    pub(crate) async fn get_text(&self, url: &str) -> Result<String, AssetError> {
        self.request_text(reqwest::Method::GET, url, None).await
    }
    pub(crate) async fn request_text(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<String, AssetError> {
        let url = Url::parse(url).map_err(|e| AssetError::new("URL", e))?;
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("play.google.com" | "itunes.apple.com"))
        {
            return Err(AssetError::new("store", "invalid host"));
        }
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| AssetError::new("permit", e))?;
        let mut request = self
            .client
            .request(method, url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
        if let Some(body) = body {
            request = request
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded;charset=UTF-8",
                )
                .body(body);
        }
        let response = request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| AssetError::new("store", e))?;
        String::from_utf8(read_body(response).await?).map_err(|e| AssetError::new("UTF8", e))
    }
}
async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, AssetError> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_ASSET_BYTES as u64)
    {
        return Err(AssetError::new("asset", "too large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AssetError::new("body", e))?
    {
        if bytes.len() + chunk.len() > MAX_ASSET_BYTES {
            return Err(AssetError::new("asset", "too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
