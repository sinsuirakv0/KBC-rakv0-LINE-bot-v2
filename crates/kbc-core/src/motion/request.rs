//! `ut`と`tut`のMotion引数を有限な型付きRequestへ変換する。

const MAX_VIDEO_FRAMES: usize = 900;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MotionFormat {
    Png,
    Mp4,
    Gif,
}

impl MotionFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Mp4 => "mp4",
            Self::Gif => "gif",
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MotionKind {
    Attack,
    Move,
    Idle,
    Knockback,
}

impl MotionKind {
    pub(crate) fn asset_index(self) -> u8 {
        match self {
            Self::Move => 0,
            Self::Idle => 1,
            Self::Attack => 2,
            Self::Knockback => 3,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameRange {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl FrameRange {
    pub(crate) fn len(self) -> usize {
        (u64::from(self.end) - u64::from(self.start) + 1) as usize
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MotionSegment {
    pub(crate) motion: MotionKind,
    pub(crate) range: Option<FrameRange>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct MotionRequest {
    pub(crate) format: MotionFormat,
    pub(crate) full: bool,
    pub(crate) form: Option<String>,
    pub(crate) segments: Vec<MotionSegment>,
}

pub(crate) fn parse_motion_arguments(
    arguments: &[String],
    forms: &[&str],
) -> Option<MotionRequest> {
    let tokens = arguments
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let format = match tokens.first()?.as_str() {
        "png" => MotionFormat::Png,
        "mp4" => MotionFormat::Mp4,
        "gif" => MotionFormat::Gif,
        _ => return None,
    };
    if tokens
        .iter()
        .filter(|value| value.as_str() == "--full")
        .count()
        > 1
    {
        return None;
    }
    let full = tokens.iter().any(|value| value == "--full");
    let normalized = tokens
        .into_iter()
        .filter(|value| value != "--full")
        .collect::<Vec<_>>();
    let mut cursor = 1;
    let form = normalized.get(cursor).and_then(|candidate| {
        forms
            .iter()
            .any(|form| candidate == form)
            .then(|| candidate.clone())
    });
    if form.is_some() {
        cursor += 1;
    }

    if format == MotionFormat::Png {
        let motion = parse_motion_kind(normalized.get(cursor)?)?;
        let frame = match normalized.get(cursor + 1) {
            Some(value) => parse_non_negative_integer(value)?,
            None => 0,
        };
        if normalized.len() != cursor + 1 + usize::from(normalized.get(cursor + 1).is_some()) {
            return None;
        }
        return Some(MotionRequest {
            format,
            full,
            form,
            segments: vec![MotionSegment {
                motion,
                range: Some(FrameRange {
                    start: frame,
                    end: frame,
                }),
            }],
        });
    }

    let mut segments = Vec::new();
    let mut explicit_frames = 0usize;
    while cursor < normalized.len() {
        let motion = parse_motion_kind(&normalized[cursor])?;
        cursor += 1;
        if cursor >= normalized.len() || parse_motion_kind(&normalized[cursor]).is_some() {
            segments.push(MotionSegment {
                motion,
                range: None,
            });
            continue;
        }
        let range = if let Some(range) = parse_compact_range(&normalized[cursor]) {
            cursor += 1;
            range
        } else {
            let start = parse_non_negative_integer(&normalized[cursor])?;
            let end = parse_non_negative_integer(normalized.get(cursor + 1)?)?;
            cursor += 2;
            (start <= end).then_some(FrameRange { start, end })?
        };
        explicit_frames = explicit_frames.checked_add(range.len())?;
        if explicit_frames > MAX_VIDEO_FRAMES {
            return None;
        }
        segments.push(MotionSegment {
            motion,
            range: Some(range),
        });
    }
    (!segments.is_empty()).then_some(MotionRequest {
        format,
        full,
        form,
        segments,
    })
}

fn parse_motion_kind(value: &str) -> Option<MotionKind> {
    match value {
        "a" => Some(MotionKind::Attack),
        "w" => Some(MotionKind::Move),
        "i" => Some(MotionKind::Idle),
        "k" => Some(MotionKind::Knockback),
        _ => None,
    }
}

fn parse_non_negative_integer(value: &str) -> Option<u32> {
    (!value.is_empty() && value.bytes().all(|value| value.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn parse_compact_range(value: &str) -> Option<FrameRange> {
    let (start, end) = value.split_once("~~")?;
    let start = parse_non_negative_integer(start)?;
    let end = parse_non_negative_integer(end)?;
    (start <= end).then_some(FrameRange { start, end })
}
