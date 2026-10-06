//! Discordの有限生成APIを使い、完成ファイルだけをLINEの作業領域へ受け取る。

use super::{MotionPlan, request::MotionFormat};
use crate::{
    Result,
    assets::AssetService,
    media::{Artifact, MAX_OUTPUT_BYTES, RenderContext},
};
use reqwest::{Method, Response, Url};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub(crate) struct RemoteMotion {
    endpoint: Url,
    secret: String,
}

impl RemoteMotion {
    pub(crate) fn from_config(url: Option<String>, secret: Option<String>) -> Result<Option<Self>> {
        let (url, secret) = match (url, secret) {
            (None, None) => return Ok(None),
            (Some(url), Some(secret)) => (url, secret),
            _ => return Err("MotionRemoteSettingsRequiredTogether".into()),
        };
        let mut endpoint = Url::parse(&url).map_err(|_| "InvalidMotionRemoteUrl")?;
        let local = matches!(
            endpoint.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        );
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && local))
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint
                .path()
                .trim_end_matches('/')
                .ends_with("/motion-jobs")
        {
            return Err("InvalidMotionRemoteUrl".into());
        }
        if !(32..=256).contains(&secret.len()) || !secret.bytes().all(|b| (33..=126).contains(&b)) {
            return Err("InvalidMotionRemoteSecret".into());
        }
        let path = endpoint.path().trim_end_matches('/').to_owned();
        endpoint.set_path(&path);
        Ok(Some(Self { endpoint, secret }))
    }

    fn job_url(&self, id: &str, artifact: bool) -> Url {
        let mut url = self.endpoint.clone();
        url.set_path(&format!(
            "{}/{id}{}",
            self.endpoint.path(),
            if artifact { "/artifact" } else { "" }
        ));
        url
    }

    pub(crate) async fn render(
        &self,
        id: &str,
        plan: &MotionPlan,
        assets: &AssetService,
        context: &RenderContext,
    ) -> Result<Artifact> {
        let body = serde_json::to_vec(
            &json!({"protocolVersion":1,"requestId":id,"assetRevision":assets.revision(),"plan":plan}),
        )?;
        if body.len() > 16 * 1024 {
            return Err("MotionRemoteRequestTooLarge".into());
        }
        let mut cleanup = true;
        let work = async {
            match assets
                .motion_request(
                    Method::POST,
                    self.endpoint.clone(),
                    &self.secret,
                    Some(body),
                )
                .await
            {
                Ok((response, _permit)) => {
                    if !response.status().is_success() {
                        cleanup = false;
                        return Err("MotionRemoteRejected".into());
                    }
                    let reply = read_json(response).await?;
                    if reply["protocolVersion"] != 1 || reply["status"] != "accepted" {
                        return Err("InvalidMotionRemoteResponse".into());
                    }
                }
                // 受付応答が消失しても、同じIDの結果を照会し二重依頼を避ける。
                Err(_) => eprintln!(
                    "Remote motion acceptance response unavailable; checking request status"
                ),
            }
            let mut failures = 0;
            loop {
                let read_status = async {
                    let (response, _permit) = assets
                        .motion_request(Method::GET, self.job_url(id, false), &self.secret, None)
                        .await?;
                    if !response.status().is_success() {
                        return Err("MotionRemoteStatusUnavailable".into());
                    }
                    read_json(response).await
                }
                .await;
                match read_status {
                    Ok(status) => {
                        failures = 0;
                        if status["protocolVersion"] != 1 {
                            return Err("InvalidMotionRemoteResponse".into());
                        }
                        match status["status"].as_str() {
                            Some("pending") => {}
                            Some("ready") => return self.download(id, plan, assets, context).await,
                            Some("failed") => return Err("MotionRemoteGenerationFailed".into()),
                            _ => return Err("InvalidMotionRemoteResponse".into()),
                        }
                    }
                    Err(_) => {
                        failures += 1;
                        if failures >= 3 {
                            return Err("MotionRemoteUnavailable".into());
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        };
        let result = tokio::select! {
            biased;
            _ = context.cancellation().cancelled() => Err("MotionRemoteCancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(1230), work) => result.unwrap_or_else(|_| Err("MotionRemoteTimeout".into())),
        };
        // 成功時は成果を削除、取消・失敗時は代行生成へ取消を伝える。通信不能時も有限に戻る。
        if cleanup {
            let _ = tokio::time::timeout(
                Duration::from_secs(3),
                assets.motion_request(Method::DELETE, self.job_url(id, false), &self.secret, None),
            )
            .await;
        }
        result
    }

    async fn download(
        &self,
        id: &str,
        plan: &MotionPlan,
        assets: &AssetService,
        context: &RenderContext,
    ) -> Result<Artifact> {
        let (mut response, _permit) = assets
            .motion_request(Method::GET, self.job_url(id, true), &self.secret, None)
            .await?;
        let content_type = match plan.format {
            MotionFormat::Png => "image/png",
            MotionFormat::Gif => "image/gif",
            MotionFormat::Mp4 => "video/mp4",
        };
        let file_name = format!("{}.{}", plan.filename_stem, plan.format.extension());
        if !response.status().is_success()
            || response
                .headers()
                .get("x-motion-protocol-version")
                .and_then(|s| s.to_str().ok())
                != Some("1")
            || response
                .headers()
                .get("content-type")
                .and_then(|s| s.to_str().ok())
                != Some(content_type)
            || response
                .headers()
                .get("x-motion-file-name")
                .and_then(|s| s.to_str().ok())
                != Some(file_name.as_str())
            || response
                .content_length()
                .is_some_and(|len| len == 0 || len > MAX_OUTPUT_BYTES)
        {
            return Err("InvalidMotionRemoteArtifact".into());
        }
        let duration_ms = if plan.format == MotionFormat::Mp4 {
            Some(
                response
                    .headers()
                    .get("x-motion-duration-ms")
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|n| (1..=30000).contains(n))
                    .ok_or("InvalidMotionRemoteDuration")?,
            )
        } else {
            None
        };
        let output = context.output_path("remote-output")?;
        let mut file = tokio::fs::File::create(&output).await?;
        let mut length = 0u64;
        let mut prefix = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            length += chunk.len() as u64;
            if length > MAX_OUTPUT_BYTES {
                return Err("MotionRemoteByteLimit".into());
            }
            prefix.extend_from_slice(&chunk[..chunk.len().min(12 - prefix.len())]);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        let valid = match plan.format {
            MotionFormat::Png => prefix.starts_with(b"\x89PNG\r\n\x1a\n"),
            MotionFormat::Gif => prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a"),
            MotionFormat::Mp4 => prefix.get(4..8) == Some(b"ftyp"),
        };
        if length == 0 || !valid {
            return Err("InvalidMotionRemoteArtifact".into());
        }
        Ok(Artifact::new(
            output,
            file_name,
            Some(content_type.into()),
            duration_ms,
        ))
    }
}

async fn read_json(mut response: Response) -> Result<Value> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 16 * 1024 {
            return Err("MotionRemoteResponseTooLarge".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
