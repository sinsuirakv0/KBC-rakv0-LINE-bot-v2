//! Motion処理内部の失敗理由を利用者向け分類と詳細へ分ける。

use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub(crate) struct MotionError {
    public_message: &'static str,
    detail: String,
}

impl MotionError {
    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        Self {
            public_message: "❌ モーションデータが正しくありません",
            detail: detail.into(),
        }
    }

    pub(crate) fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            public_message: "❌ モーション素材を取得できませんでした",
            detail: detail.into(),
        }
    }

    pub(crate) fn render(detail: impl Into<String>) -> Self {
        Self {
            public_message: "❌ モーションの生成に失敗しました",
            detail: detail.into(),
        }
    }

    pub(crate) fn public_message(&self) -> &'static str {
        self.public_message
    }
}

impl Display for MotionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for MotionError {}
