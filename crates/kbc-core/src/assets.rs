use crate::asset_platform::{AssetPlatform, INDEX_LIMIT, RAW_BASE, ResolvedAsset, split_url};
use reqwest::{Client, Url, redirect::Policy};
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

pub const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct AssetService {
    base: String,
    revision: String,
    client: Client,
    permits: Arc<Semaphore>,
    platform: Arc<AssetPlatform>,
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
            revision: commit.into(),
            base: format!(
                "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{commit}/jp/sitedata"
            ),
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(Policy::none())
                .build()?,
            permits: Arc::new(Semaphore::new(2)),
            platform: Arc::new(AssetPlatform::default()),
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
            revision: revision.into(),
            base: format!(
                "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{revision}/jp/sitedata"
            ),
            client: self.client.clone(),
            permits: Arc::clone(&self.permits),
            platform: Arc::clone(&self.platform),
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
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-@".contains(&byte))
            })
        {
            return Err(AssetError::new("asset", "invalid path"));
        }
        Url::parse(&format!("{}/{path}", self.base)).map_err(|e| AssetError::new("asset", e))
    }

    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }
    pub(crate) async fn pinned(&self) -> Result<Self, AssetError> {
        let dataset = self
            .platform
            .load(&self.revision, |url, limit| {
                self.repository_read(url, limit)
            })
            .await
            .map_err(|e| AssetError::new("dataset", e))?;
        self.at_revision(&dataset.revision)
            .map_err(|e| AssetError::new("dataset", e))
    }

    // 代行APIも既存ClientとHTTP枠を共有し、応答本文の処理中は枠を保持する。
    pub(crate) async fn motion_request(
        &self,
        method: reqwest::Method,
        url: Url,
        secret: &str,
        body: Option<Vec<u8>>,
    ) -> crate::Result<(reqwest::Response, tokio::sync::OwnedSemaphorePermit)> {
        let permit = Arc::clone(&self.permits).acquire_owned().await?;
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(secret)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::CACHE_CONTROL, "no-store");
        if let Some(body) = body {
            request = request.body(body);
        }
        Ok((request.send().await?, permit))
    }
    pub async fn exists(&self, path: &str) -> Result<bool, AssetError> {
        let url = self.url(path)?;
        let Some(resolved) = self.resolve(&self.revision, path).await? else {
            return Ok(false);
        };
        if resolved.proof.is_some() {
            return Ok(true);
        }
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
        let prefix = format!("{}/", self.base);
        let path = url
            .as_str()
            .strip_prefix(&prefix)
            .ok_or_else(|| AssetError::new("asset", "invalid path"))?;
        self.asset_bytes(&self.revision, path, MAX_ASSET_BYTES)
            .await?
            .ok_or_else(|| AssetError::new("asset", "not found"))
    }
    async fn resolve(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<Option<ResolvedAsset>, AssetError> {
        self.platform
            .load(revision, |url, limit| self.repository_read(url, limit))
            .await
            .map_err(|e| AssetError::new("dataset", e))?
            .resolve(path)
            .map_err(|e| AssetError::new("dataset", e))
    }
    pub(crate) async fn resolved_url(&self, path: &str) -> Result<String, AssetError> {
        self.resolve(&self.revision, path)
            .await?
            .map(|file| file.url)
            .ok_or_else(|| AssetError::new("asset", "not found"))
    }
    async fn asset_bytes(
        &self,
        revision: &str,
        path: &str,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, AssetError> {
        let Some(file) = self.resolve(revision, path).await? else {
            return Ok(None);
        };
        let bytes = self
            .repository_read(file.url, limit)
            .await
            .map_err(|e| AssetError::new("asset", e))?
            .ok_or_else(|| AssetError::new("asset", "payload missing"))?;
        if let Some(proof) = file.proof {
            proof
                .verify(&bytes)
                .map_err(|e| AssetError::new("asset", e))?;
        }
        Ok(Some(bytes))
    }
    async fn repository_read(&self, url: String, limit: usize) -> Result<Option<Vec<u8>>, String> {
        if limit > INDEX_LIMIT
            || !(url.starts_with(RAW_BASE)
                || url
                    .starts_with("https://api.github.com/repos/sinsuirakv0/KBC-rakv0-assets/git/"))
        {
            return Err("Invalid dataset request".into());
        }
        let _permit = self.permits.acquire().await.map_err(|e| e.to_string())?;
        let mut request = self
            .client
            .get(&url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .header(reqwest::header::USER_AGENT, "KBC-LINEbot-v2");
        if url.starts_with("https://api.github.com/")
            && let Ok(token) = std::env::var("GITHUB_TOKEN")
                .or_else(|_| std::env::var("PUSH_SUBSCRIPTIONS_GITHUB_TOKEN"))
            && !token.is_empty()
        {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|e| e.to_string())?;
        read_body_limit(response, limit)
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }
    // 素材・ストア・スケジュールでClient、HTTP上限2枠、15秒timeout、4MiB上限を共有する。
    pub(crate) async fn get_text(&self, url: &str) -> Result<String, AssetError> {
        self.request_text(reqwest::Method::GET, url, None).await
    }
    pub(crate) async fn request_text(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<String, AssetError> {
        self.text_response(method, url, body, false)
            .await?
            .ok_or_else(|| AssetError::new("GET", "missing response"))
    }
    pub(crate) async fn get_optional_text(&self, url: &str) -> Result<Option<String>, AssetError> {
        self.text_response(reqwest::Method::GET, url, None, true)
            .await
    }
    async fn text_response(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Vec<u8>>,
        allow_not_found: bool,
    ) -> Result<Option<String>, AssetError> {
        if method == reqwest::Method::GET
            && let Some((revision, path)) = split_url(url)
        {
            let bytes = self.asset_bytes(revision, path, MAX_ASSET_BYTES).await?;
            if bytes.is_none() && !allow_not_found {
                return Err(AssetError::new("asset", "not found"));
            }
            return bytes
                .map(String::from_utf8)
                .transpose()
                .map_err(|e| AssetError::new("UTF8", e));
        }
        let url = Url::parse(url).map_err(|e| AssetError::new("URL", e))?;
        if url.scheme() != "https"
            || !(matches!(url.host_str(), Some("play.google.com" | "itunes.apple.com"))
                || (url.host_str() == Some("raw.githubusercontent.com")
                    && [
                        "/sinsuirakv0/KBC-rakv0-event/",
                        "/sinsuirakv0/KBC-rakv0-assets/",
                    ]
                    .iter()
                    .any(|prefix| url.path().starts_with(prefix)))
                || (url.host_str() == Some("api.github.com")
                    && url
                        .path()
                        .starts_with("/repos/sinsuirakv0/KBC-rakv0-event/git/")))
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
            .request(method, url.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .header(reqwest::header::USER_AGENT, "KBC-LINEbot-v2");
        if url.host_str() == Some("api.github.com")
            && let Ok(token) = std::env::var("GITHUB_TOKEN")
                .or_else(|_| std::env::var("PUSH_SUBSCRIPTIONS_GITHUB_TOKEN"))
            && !token.is_empty()
        {
            request = request.bearer_auth(token);
        }
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
            .map_err(|e| AssetError::new("store", e))?;
        if allow_not_found && response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .map_err(|e| AssetError::new("store", e))?;
        String::from_utf8(read_body(response).await?)
            .map(Some)
            .map_err(|e| AssetError::new("UTF8", e))
    }
}
async fn read_body(response: reqwest::Response) -> Result<Vec<u8>, AssetError> {
    read_body_limit(response, MAX_ASSET_BYTES).await
}
async fn read_body_limit(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, AssetError> {
    if response
        .content_length()
        .is_some_and(|n| n > maximum as u64)
    {
        return Err(AssetError::new("asset", "too large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AssetError::new("body", e))?
    {
        if bytes.len() + chunk.len() > maximum {
            return Err(AssetError::new("asset", "too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod platform_live {
    use super::*;
    #[tokio::test]
    #[ignore = "公開assetsの疎通検証。LINEへ接続しない"]
    async fn asset_platform_live() {
        let revision = std::env::var("KBC_ASSET_TEST_REF").unwrap_or("main".into());
        let source = AssetService::new(&revision).unwrap();
        let pinned = source.pinned().await.unwrap();
        let chars = pinned.bytes("character-index.json").await.unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(chars.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&chars)).unwrap();
        assert!(value["units"].as_array().unwrap().len() > 800);
        for path in [
            "Data/unitbuy.csv",
            "res/Enemyname.tsv",
            "res/Map_Name.csv",
            "res/All_day_event.tsv",
            "res/Mission_Name.csv",
            "Data/GatyaData_Option_SetR.tsv",
            "ImageData/000_f.imgcut",
            "ImageData/000_f.mamodel",
            "ImageData/000_f00.maanim",
        ] {
            assert!(!pinned.bytes(path).await.unwrap().is_empty());
        }
        let image = pinned.bytes("Number/000_f.png").await.unwrap();
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(
            pinned
                .bytes("assets/close@2x.png")
                .await
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        assert!(!pinned.exists("Number/not-selected.png").await.unwrap());
        println!("LINE verified assets: {}", pinned.revision());
    }
}
