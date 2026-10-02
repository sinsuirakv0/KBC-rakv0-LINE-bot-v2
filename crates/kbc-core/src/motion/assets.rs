//! 解決済みMotion Assetを共有HTTP基盤から有限並列で取得する。

use std::collections::HashMap;

use tokio::task::JoinSet;

use crate::assets::{AssetError as RemoteDataError, AssetService as RemoteAssetSource};

use super::MotionPlan;
use super::request::MotionKind;

pub(super) struct MotionAssets {
    pub(super) sprite: Vec<u8>,
    pub(super) imgcut: Vec<u8>,
    pub(super) model: Vec<u8>,
    pub(super) animations: HashMap<MotionKind, Vec<u8>>,
}

pub(super) async fn load_motion_assets(
    plan: &MotionPlan,
    source: &RemoteAssetSource,
) -> Result<MotionAssets, RemoteDataError> {
    let (sprite, imgcut, model) = tokio::try_join!(
        source.bytes(&plan.sprite_path),
        source.bytes(&plan.imgcut_path),
        source.bytes(&plan.model_path),
    )?;
    let mut animation_loads = JoinSet::new();
    for (motion, path) in &plan.animation_paths {
        let source = source.clone();
        let motion = *motion;
        let path = path.clone();
        animation_loads.spawn(async move {
            let data = source.bytes(&path).await?;
            Ok::<_, RemoteDataError>((motion, data))
        });
    }
    let mut animations = HashMap::with_capacity(plan.animation_paths.len());
    while let Some(result) = animation_loads.join_next().await {
        let (motion, data) =
            result.map_err(|error| RemoteDataError::new("asset", error.to_string()))??;
        animations.insert(motion, data);
    }
    Ok(MotionAssets {
        sprite,
        imgcut,
        model,
        animations,
    })
}
