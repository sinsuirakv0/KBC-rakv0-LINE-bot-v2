//! 検証済みcurrentと採用元索引から、旧logical pathを同一commitの実ファイルへ解決する。
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub(crate) const RAW_BASE: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/";
pub(crate) const INDEX_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) fn split_url(url: &str) -> Option<(&str, &str)> {
    url.strip_prefix(RAW_BASE)?.split_once("/jp/sitedata/")
}
fn safe_path(path: &str) -> bool {
    path.split('/').all(|part| {
        !part.is_empty()
            && !matches!(part, "." | "..")
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    })
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|e| e.to_string())
}

#[derive(Clone, Deserialize)]
pub(crate) struct Proof {
    pub size: usize,
    pub sha256: String,
}
impl Proof {
    pub(crate) fn verify(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != self.size || hash(bytes) != self.sha256 {
            return Err("Asset size/hash mismatch".into());
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize)]
struct File {
    path: String,
    #[serde(flatten)]
    proof: Proof,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Header {
    dataset_id: String,
    build_id: String,
    source_region: String,
    version_code: u64,
    version_name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Current {
    schema_version: u32,
    status: String,
    generated_path: String,
    manifest_sha256: String,
    #[serde(flatten)]
    header: Header,
}
#[derive(Deserialize)]
struct Manifest {
    header: Header,
    artifacts: HashMap<String, Proof>,
}
#[derive(Deserialize)]
struct Index {
    header: Header,
    files: HashMap<String, File>,
}

pub(crate) struct ResolvedAsset {
    pub url: String,
    pub proof: Option<Proof>,
}
pub(crate) struct Dataset {
    pub revision: String,
    base: String,
    generated: String,
    files: Option<HashMap<String, File>>,
    characters: Option<Proof>,
}
impl Dataset {
    pub(crate) fn resolve(&self, path: &str) -> Result<Option<ResolvedAsset>, String> {
        if !safe_path(path) {
            return Err("Unsafe asset path".into());
        }
        let Some(files) = &self.files else {
            return Ok(Some(ResolvedAsset {
                url: format!("{}jp/sitedata/{path}", self.base),
                proof: None,
            }));
        };
        if path == "character-index.json" {
            return Ok(Some(ResolvedAsset {
                url: format!("{}{}/site/characters.json", self.base, self.generated),
                proof: self.characters.clone(),
            }));
        }
        Ok(files.get(path).map(|file| ResolvedAsset {
            url: format!("{}{}", self.base, file.path),
            proof: Some(file.proof.clone()),
        }))
    }
}
struct Cached {
    reference: String,
    at: Instant,
    dataset: Arc<Dataset>,
}
#[derive(Default)]
pub(crate) struct AssetPlatform {
    cached: Mutex<Option<Cached>>,
}
impl AssetPlatform {
    // HTTPは呼出側の既存Client・有限枠を借りる。本文上限だけ索引用に16MiBを許可する。
    pub(crate) async fn load<F, R>(&self, reference: &str, read: F) -> Result<Arc<Dataset>, String>
    where
        F: Fn(String, usize) -> R,
        R: Future<Output = Result<Option<Vec<u8>>, String>>,
    {
        if reference != "main"
            && (reference.len() != 40 || !reference.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("Invalid asset revision".into());
        }
        let mut cached = self.cached.lock().await;
        let started = Instant::now();
        if let Some(value) = cached.as_ref()
            && value.at.elapsed() < Duration::from_secs(120)
            && (value.reference == reference || value.dataset.revision == reference)
        {
            return Ok(Arc::clone(&value.dataset));
        }
        // 期限超過の索引を取得失敗時に再利用しない。
        let revision = if reference == "main" {
            #[derive(Deserialize)]
            struct Commit {
                sha: String,
            }
            let bytes = read(
                "https://api.github.com/repos/sinsuirakv0/KBC-rakv0-assets/git/ref/heads/main"
                    .into(),
                64 * 1024,
            )
            .await?
            .ok_or("Asset ref missing")?;
            #[derive(Deserialize)]
            struct Ref {
                object: Commit,
            }
            json::<Ref>(&bytes)?.object.sha
        } else {
            reference.to_owned()
        };
        if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid resolved revision".into());
        }
        let base = format!("{RAW_BASE}{revision}/");
        let current = read(format!("{base}metadata/jp/current.json"), 64 * 1024).await?;
        let dataset = if let Some(bytes) = current {
            let current: Current = json(&bytes)?;
            let header = &current.header;
            if current.schema_version != 1
                || current.status != "verified"
                || header.source_region != "jp"
                || !safe_path(&header.build_id)
                || header.build_id.contains('/')
                || !safe_path(&header.dataset_id)
                || header.dataset_id.contains('/')
                || current.generated_path != format!("generated/jp/datasets/{}", header.build_id)
            {
                return Err("Unverified asset dataset".into());
            }
            let manifest_bytes = read(
                format!("{base}{}/manifest.json", current.generated_path),
                64 * 1024,
            )
            .await?
            .ok_or("Asset manifest missing")?;
            if hash(&manifest_bytes) != current.manifest_sha256 {
                return Err("Asset manifest hash mismatch".into());
            }
            let manifest: Manifest = json(&manifest_bytes)?;
            if manifest.header != *header {
                return Err("Asset header mismatch".into());
            }
            let proof = manifest
                .artifacts
                .get("indexes/assets.json")
                .ok_or("Asset index descriptor missing")?;
            if proof.size > INDEX_LIMIT {
                return Err("Asset index too large".into());
            }
            let bytes = read(
                format!("{base}{}/indexes/assets.json", current.generated_path),
                INDEX_LIMIT,
            )
            .await?
            .ok_or("Asset index missing")?;
            proof.verify(&bytes)?;
            let index: Index = json(&bytes)?;
            if index.header != *header || index.files.is_empty() || index.files.len() > 131072 {
                return Err("Invalid asset index".into());
            }
            let prefixes = [
                format!("jp/Local/{}/", header.dataset_id),
                "jp/server/".into(),
                "overrides/jp/assets/".into(),
            ];
            for (name, file) in &index.files {
                if !safe_path(name)
                    || !safe_path(&file.path)
                    || !prefixes.iter().any(|p| file.path.starts_with(p))
                {
                    return Err("Asset outside verified dataset".into());
                }
            }
            Dataset {
                revision,
                base,
                generated: current.generated_path,
                files: Some(index.files),
                characters: Some(
                    manifest
                        .artifacts
                        .get("site/characters.json")
                        .ok_or("Character descriptor missing")?
                        .clone(),
                ),
            }
        } else if reference != "main" {
            // 移行前commitの既存fixture・保存済みjobだけは、そのcommitの旧実体を読む。
            Dataset {
                revision,
                base,
                generated: String::new(),
                files: None,
                characters: None,
            }
        } else {
            return Err("Verified current missing".into());
        };
        let dataset = Arc::new(dataset);
        *cached = Some(Cached {
            reference: reference.into(),
            at: started,
            dataset: Arc::clone(&dataset),
        });
        Ok(dataset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::future::ready;

    #[tokio::test]
    async fn verified_platform_rejects_corruption_and_expired_stale() {
        let revision = "a".repeat(40);
        let base = format!("{RAW_BASE}{revision}/");
        let header = json!({"datasetId":"jp-1", "buildId":"build-1", "sourceRegion":"jp", "versionName":"1", "versionCode":1});
        let bytes = |v: serde_json::Value| serde_json::to_vec(&v).unwrap();
        let index = bytes(
            json!({"header":header, "files":{"Number/000_f.png":{"path":"overrides/jp/assets/Number/000_f.png", "size":3,"sha256":hash(b"png")}}}),
        );
        let manifest = bytes(
            json!({"header":header,"artifacts":{"indexes/assets.json":{"size":index.len(),"sha256":hash(&index)},"site/characters.json":{"size":2,"sha256":hash(b"{}")}}}),
        );
        let current = bytes(
            json!({"schemaVersion":1,"status":"verified","sourceRegion":"jp","datasetId":"jp-1","buildId":"build-1","versionName":"1","versionCode":1,"generatedPath":"generated/jp/datasets/build-1","manifestSha256":hash(&manifest)}),
        );
        let mut files = HashMap::from([
            (format!("{base}metadata/jp/current.json"), current),
            (
                format!("{base}generated/jp/datasets/build-1/manifest.json"),
                manifest,
            ),
            (
                format!("{base}generated/jp/datasets/build-1/indexes/assets.json"),
                index,
            ),
        ]);
        let platform = AssetPlatform::default();
        let dataset = platform
            .load(&revision, |url, _| ready(Ok(files.get(&url).cloned())))
            .await
            .unwrap();
        let selected = dataset.resolve("Number/000_f.png").unwrap().unwrap();
        assert!(
            selected
                .url
                .ends_with("overrides/jp/assets/Number/000_f.png")
        );
        selected.proof.unwrap().verify(b"png").unwrap();
        assert!(dataset.resolve("Number/missing.png").unwrap().is_none());
        assert!(dataset.resolve("../escape").is_err());
        assert!(
            dataset
                .resolve("character-index.json")
                .unwrap()
                .unwrap()
                .url
                .ends_with("site/characters.json")
        );
        files
            .get_mut(&format!(
                "{base}generated/jp/datasets/build-1/indexes/assets.json"
            ))
            .unwrap()
            .push(b' ');
        assert!(
            AssetPlatform::default()
                .load(&revision, |url, _| ready(Ok(files.get(&url).cloned())))
                .await
                .is_err()
        );
        platform.cached.lock().await.as_mut().unwrap().at -= Duration::from_secs(121);
        assert!(
            platform
                .load(&revision, |_, _| ready(Err("offline".into())))
                .await
                .is_err()
        );
    }
}
