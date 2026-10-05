//! にゃんこ大戦争のストア公開バージョンを取得する。

use std::cmp::Ordering;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::now_ms;
use reqwest::{Method, Url};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::assets::AssetService;

const PACKAGE_ID: &str = "jp.co.ponos.battlecats";
const GOOGLE_PLAY_PAGE_URL: &str =
    "https://play.google.com/store/apps/details?id=jp.co.ponos.battlecats&hl=ja&gl=JP";
const GOOGLE_PLAY_RPC_URL: &str =
    "https://play.google.com/_/PlayStoreUi/data/batchexecute?hl=ja&gl=JP";
const APP_STORE_LOOKUP_URL: &str =
    "https://itunes.apple.com/lookup?bundleId=jp.co.ponos.battlecats&country=jp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorePlatform {
    Android,
    Ios,
}

impl StorePlatform {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Android => "Android（Google Play）",
            Self::Ios => "iOS（App Store）",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }

    pub(crate) fn store_name(self) -> &'static str {
        match self {
            Self::Android => "Google Play",
            Self::Ios => "App Store",
        }
    }

    pub(crate) fn store_url(self) -> &'static str {
        match self {
            Self::Android => "https://play.google.com/store/apps/details?id=jp.co.ponos.battlecats",
            Self::Ios => "https://apps.apple.com/jp/app/id547145938",
        }
    }
}

pub(crate) struct StoreVersionSource {
    inner: StoreVersionSourceInner,
}

enum StoreVersionSourceInner {
    GooglePlay(GooglePlaySource),
    AppStore(AppStoreSource),
}

impl StoreVersionSource {
    pub(crate) fn new(platform: StorePlatform, http: Arc<AssetService>) -> Self {
        let inner = match platform {
            StorePlatform::Android => {
                StoreVersionSourceInner::GooglePlay(GooglePlaySource { http, rpc: None })
            }
            StorePlatform::Ios => StoreVersionSourceInner::AppStore(AppStoreSource { http }),
        };
        Self { inner }
    }

    pub(crate) async fn fetch(&mut self) -> Result<String, StoreUpdateError> {
        match &mut self.inner {
            StoreVersionSourceInner::GooglePlay(source) => source.fetch().await,
            StoreVersionSourceInner::AppStore(source) => source.fetch().await,
        }
    }
}

struct GooglePlaySource {
    http: Arc<AssetService>,
    rpc: Option<GoogleRpc>,
}

impl GooglePlaySource {
    async fn fetch(&mut self) -> Result<String, StoreUpdateError> {
        for attempt in 0..2 {
            if self.rpc.is_none() {
                let html = self
                    .http
                    .get_text(GOOGLE_PLAY_PAGE_URL)
                    .await
                    .map_err(|error| StoreUpdateError::detail("google-bootstrap-request", error))?;
                self.rpc = Some(parse_google_rpc(&html)?);
            }
            let result =
                fetch_google_version(&self.http, self.rpc.as_ref().expect("RPC is initialized"))
                    .await;
            if result.is_ok() || attempt == 1 {
                return result;
            }
            self.rpc = None;
        }
        Err(StoreUpdateError::new("google-request"))
    }
}

struct AppStoreSource {
    http: Arc<AssetService>,
}

impl AppStoreSource {
    async fn fetch(&self) -> Result<String, StoreUpdateError> {
        let url = format!("{APP_STORE_LOOKUP_URL}&_={}", now_ms());
        let body = self
            .http
            .get_text(&url)
            .await
            .map_err(|error| StoreUpdateError::detail("apple-request", error))?;
        parse_apple_version(&body)
    }
}

#[derive(Clone)]
struct GoogleRpc {
    id: String,
    request: String,
}

async fn fetch_google_version(
    http: &AssetService,
    rpc: &GoogleRpc,
) -> Result<String, StoreUpdateError> {
    let payload = json!([[[rpc.id, rpc.request, Value::Null, "generic"]]]).to_string();
    let mut form_url = Url::parse("https://localhost/").expect("form URL is valid");
    form_url.query_pairs_mut().append_pair("f.req", &payload);
    let body = form_url
        .query()
        .expect("form contains f.req")
        .as_bytes()
        .to_vec();
    let url = format!("{GOOGLE_PLAY_RPC_URL}&rpcids={}", rpc.id);
    let body = http
        .request_text(Method::POST, &url, Some(body))
        .await
        .map_err(|error| StoreUpdateError::detail("google-request", error))?;
    parse_google_response(&body, &rpc.id)
}

fn parse_google_rpc(html: &str) -> Result<GoogleRpc, StoreUpdateError> {
    let entry = html
        .find("'ds:5'")
        .map(|index| &html[index..])
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let id_start = entry
        .find("id:'")
        .map(|index| index + "id:'".len())
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let id_end = entry[id_start..]
        .find('\'')
        .map(|index| id_start + index)
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let id = &entry[id_start..id_end];
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(StoreUpdateError::new("google-bootstrap-schema"));
    }
    let request_marker = entry[id_end..]
        .find("request:")
        .map(|index| id_end + index + "request:".len())
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let request_start = entry[request_marker..]
        .find('[')
        .map(|index| request_marker + index)
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let request_end = json_array_end(entry, request_start)
        .ok_or_else(|| StoreUpdateError::new("google-bootstrap-schema"))?;
    let request = &entry[request_start..request_end];
    serde_json::from_str::<Value>(request)
        .map_err(|_| StoreUpdateError::new("google-bootstrap-schema"))?;
    Ok(GoogleRpc {
        id: id.to_owned(),
        request: request.to_owned(),
    })
}

fn json_array_end(value: &str, start: usize) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in value[start..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(start + offset + character.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_google_response(body: &str, rpc_id: &str) -> Result<String, StoreUpdateError> {
    let payload = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find_map(|value| find_google_payload(&value, rpc_id))
        .ok_or_else(|| StoreUpdateError::new("google-response-schema"))?;
    let value: Value = serde_json::from_str(&payload)
        .map_err(|_| StoreUpdateError::new("google-response-schema"))?;
    let version = value
        .pointer("/1/2/140/0/0/0")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreUpdateError::new("google-version-missing"))?;
    validated_version(version)
}

fn find_google_payload(value: &Value, rpc_id: &str) -> Option<String> {
    let values = value.as_array()?;
    if values.len() >= 3
        && values.first()?.as_str() == Some("wrb.fr")
        && values.get(1)?.as_str() == Some(rpc_id)
    {
        return values.get(2)?.as_str().map(str::to_owned);
    }
    values
        .iter()
        .find_map(|value| find_google_payload(value, rpc_id))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleLookup {
    result_count: usize,
    results: Vec<AppleResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleResult {
    bundle_id: String,
    version: String,
}

fn parse_apple_version(body: &str) -> Result<String, StoreUpdateError> {
    let lookup: AppleLookup =
        serde_json::from_str(body).map_err(|_| StoreUpdateError::new("apple-response-schema"))?;
    if lookup.result_count != 1 || lookup.results.len() != 1 {
        return Err(StoreUpdateError::new("apple-app-missing"));
    }
    let result = &lookup.results[0];
    if result.bundle_id != PACKAGE_ID {
        return Err(StoreUpdateError::new("apple-bundle-mismatch"));
    }
    validated_version(&result.version)
}

fn validated_version(value: &str) -> Result<String, StoreUpdateError> {
    parse_version(value)
        .map(|_| value.to_owned())
        .ok_or_else(|| StoreUpdateError::new("invalid-store-version"))
}

pub(crate) fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let mut left = parse_version(left)?;
    let mut right = parse_version(right)?;
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    Some(left.cmp(&right))
}

pub(crate) fn valid_version(value: &str) -> bool {
    parse_version(value).is_some()
}

pub(crate) fn asset_version_code(value: &str) -> Option<String> {
    let parts = parse_version(value)?;
    let major = *parts.first()?;
    let minor = *parts.get(1).unwrap_or(&0);
    let patch = *parts.get(2).unwrap_or(&0);
    (parts.len() <= 3 && minor <= 99 && patch <= 99).then(|| format!("{major}{minor:02}{patch:02}"))
}

fn parse_version(value: &str) -> Option<Vec<u32>> {
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let parts = value
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.chars().all(|character| character.is_ascii_digit()))
                .then(|| part.parse::<u32>().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then_some(parts)
}

#[derive(Debug)]
pub(crate) struct StoreUpdateError {
    code: &'static str,
}

impl StoreUpdateError {
    fn new(code: &'static str) -> Self {
        Self { code }
    }

    fn detail(code: &'static str, error: impl Display) -> Self {
        eprintln!("Store version request failed ({code}): {error}");
        Self::new(code)
    }
}

impl Display for StoreUpdateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for StoreUpdateError {}

#[cfg(test)]
mod tests {
    use super::{
        asset_version_code, compare_versions, parse_apple_version, parse_google_response,
        parse_google_rpc,
    };
    use std::cmp::Ordering;

    #[test]
    fn parses_google_bootstrap_and_response() {
        let html = "x 'ds:5' : {id:'Ws7gDc',request:[null,[\"a]\"],[1]]},'ds:6' y";
        let rpc = parse_google_rpc(html).expect("RPC should parse");
        assert_eq!(rpc.id, "Ws7gDc");
        assert_eq!(rpc.request, "[null,[\"a]\"],[1]]");

        let mut fields = vec![serde_json::Value::Null; 141];
        fields[140] = serde_json::json!([[["15.6.1"]]]);
        let app = serde_json::json!([null, [null, null, fields]]);
        let inner = app.to_string();
        let frame = serde_json::json!([["wrb.fr", "Ws7gDc", inner]]).to_string();
        assert_eq!(
            parse_google_response(&format!(")]}}'\n{frame}"), "Ws7gDc").unwrap(),
            "15.6.1"
        );
    }

    #[test]
    fn parses_apple_lookup_and_compares_patch_versions() {
        let body = r#"{"resultCount":1,"results":[{"bundleId":"jp.co.ponos.battlecats","version":"15.6.1"}]}"#;
        assert_eq!(parse_apple_version(body).unwrap(), "15.6.1");
        assert_eq!(
            compare_versions("15.6.1", "15.6.0"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_versions("15.6", "15.6.0"), Some(Ordering::Equal));
        assert_eq!(asset_version_code("15.7.0").as_deref(), Some("150700"));
        assert_eq!(asset_version_code("15.6.1").as_deref(), Some("150601"));
    }
}
