//! 検索結果から解決済みAssetと出力条件を保持する。

use std::collections::HashMap;

use super::request::{MotionFormat, MotionKind, MotionSegment};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct MotionPlan {
    pub(crate) format: MotionFormat,
    pub(crate) full: bool,
    pub(crate) filename_stem: String,
    pub(crate) preview_scale: f32,
    pub(crate) segments: Vec<MotionSegment>,
    pub(crate) sprite_path: String,
    pub(crate) imgcut_path: String,
    pub(crate) model_path: String,
    pub(crate) animation_paths: HashMap<MotionKind, String>,
}
