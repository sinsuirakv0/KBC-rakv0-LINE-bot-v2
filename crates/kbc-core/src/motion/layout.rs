//! 全Frameの可視範囲を集約し、有限な偶数Sizeの出力Layoutを決定する。

use std::collections::{HashMap, HashSet};

use super::MotionError;
use super::evaluate::{DrawPacket, evaluate};
use super::plan::MotionPlan;
use super::project::MotionProject;
use super::raster::SpriteSheet;
use super::request::MotionKind;

const MAX_VIDEO_FRAMES: usize = 900;
const MAX_DIMENSION: f32 = 960.0;
const MAX_IMAGE_PIXELS: f32 = 640.0 * 480.0;
const MAX_VIDEO_PIXELS: f32 = 480.0 * 400.0;
const PADDING: f32 = 8.0;
const VIEWPORT_SIDE_MARGIN: f32 = 1.0;
const VIEWPORT_TOP_MARGIN: f32 = 0.35;
const VIEWPORT_BOTTOM_MARGIN: f32 = 0.1;

#[derive(Clone, Copy, Debug)]
pub(super) struct FrameRef {
    pub(super) motion: MotionKind,
    pub(super) frame: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) scale: f32,
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
}

#[derive(Clone, Copy)]
struct Bounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            left: f32::INFINITY,
            top: f32::INFINITY,
            right: f32::NEG_INFINITY,
            bottom: f32::NEG_INFINITY,
        }
    }

    fn include(&mut self, other: Self) {
        self.left = self.left.min(other.left);
        self.top = self.top.min(other.top);
        self.right = self.right.max(other.right);
        self.bottom = self.bottom.max(other.bottom);
    }

    fn valid(self) -> bool {
        self.left.is_finite()
            && self.top.is_finite()
            && self.right.is_finite()
            && self.bottom.is_finite()
            && self.left < self.right
            && self.top < self.bottom
    }
}

pub(super) fn resolve_frames(
    plan: &MotionPlan,
    project: &MotionProject,
) -> Result<Vec<FrameRef>, MotionError> {
    let mut frames = Vec::new();
    for segment in &plan.segments {
        let maximum = project
            .max_frame(segment.motion)
            .ok_or_else(|| MotionError::invalid("requested animation is missing"))?;
        let range = segment.range.unwrap_or(super::request::FrameRange {
            start: 0,
            end: maximum,
        });
        if range.end > maximum {
            return Err(MotionError::invalid(format!(
                "frame {} exceeds animation maximum {maximum}",
                range.end
            )));
        }
        for frame in range.start..=range.end {
            frames.push(FrameRef {
                motion: segment.motion,
                frame,
            });
            if frames.len() > MAX_VIDEO_FRAMES {
                return Err(MotionError::invalid("rendered frame count exceeds 900"));
            }
        }
    }
    if frames.is_empty() {
        Err(MotionError::invalid("no frames were selected"))
    } else {
        Ok(frames)
    }
}

pub(super) fn measure(
    plan: &MotionPlan,
    project: &MotionProject,
    sprite: &SpriteSheet,
    frames: &[FrameRef],
    mut on_frame: impl FnMut(usize) -> Result<(), MotionError>,
) -> Result<Layout, MotionError> {
    let mut parts = HashMap::<usize, Bounds>::new();
    let mut reference = Bounds::empty();
    let mut reference_motions = HashSet::new();
    for frame in frames {
        if reference_motions.insert(frame.motion) {
            for packet in evaluate(project, frame.motion, 0)? {
                if let Some(bounds) = packet_bounds(sprite, &packet)? {
                    reference.include(bounds);
                }
            }
        }
    }
    for (index, frame) in frames.iter().enumerate() {
        for packet in evaluate(project, frame.motion, frame.frame)? {
            if let Some(bounds) = packet_bounds(sprite, &packet)? {
                parts
                    .entry(packet.part_index)
                    .or_insert_with(Bounds::empty)
                    .include(bounds);
            }
        }
        on_frame(index + 1)?;
    }
    let mut bounds = Bounds::empty();
    for part in parts.values().copied() {
        bounds.include(part);
    }
    if !plan.full && reference.valid() && bounds.valid() {
        let span = (reference.right - reference.left).max(reference.bottom - reference.top);
        let limit = Bounds {
            left: reference.left - span * VIEWPORT_SIDE_MARGIN,
            right: reference.right + span * VIEWPORT_SIDE_MARGIN,
            top: reference.top - span * VIEWPORT_TOP_MARGIN,
            bottom: reference.bottom + span * VIEWPORT_BOTTOM_MARGIN,
        };
        let cropped = Bounds {
            left: bounds.left.max(limit.left),
            top: bounds.top.max(limit.top),
            right: bounds.right.min(limit.right),
            bottom: bounds.bottom.min(limit.bottom),
        };
        if cropped.valid() {
            bounds = cropped;
        }
    }
    if !bounds.valid() {
        bounds = Bounds {
            left: -16.0,
            top: -32.0,
            right: 16.0,
            bottom: 0.0,
        };
    }
    finish_layout(plan, bounds)
}

fn packet_bounds(sprite: &SpriteSheet, packet: &DrawPacket) -> Result<Option<Bounds>, MotionError> {
    if packet.opacity <= 0.0 {
        return Ok(None);
    }
    let Some(visible) = sprite.visible_bounds(packet.cut_index, packet.blend_mode)? else {
        return Ok(None);
    };
    let vertices = packet.vertices;
    let dx = [
        vertices[3][0] - vertices[0][0],
        vertices[3][1] - vertices[0][1],
    ];
    let dy = [
        vertices[1][0] - vertices[0][0],
        vertices[1][1] - vertices[0][1],
    ];
    let mut bounds = Bounds::empty();
    for x in [visible[0], visible[2]] {
        for y in [visible[1], visible[3]] {
            let point_x = vertices[0][0] + x * dx[0] + y * dy[0];
            let point_y = vertices[0][1] + x * dx[1] + y * dy[1];
            bounds.include(Bounds {
                left: point_x,
                top: point_y,
                right: point_x,
                bottom: point_y,
            });
        }
    }
    if [bounds.left, bounds.top, bounds.right, bounds.bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        Ok(bounds.valid().then_some(bounds))
    } else {
        Err(MotionError::invalid("motion geometry is not finite"))
    }
}

fn finish_layout(plan: &MotionPlan, bounds: Bounds) -> Result<Layout, MotionError> {
    let source_width = (bounds.right - bounds.left).max(1.0);
    let source_height = (bounds.bottom - bounds.top).max(1.0);
    let padding = PADDING * 2.0;
    let pixel_ratio = if plan.format == super::request::MotionFormat::Png {
        4.0
    } else {
        1.0
    };
    let max_pixels = if plan.format == super::request::MotionFormat::Png {
        MAX_IMAGE_PIXELS
    } else {
        MAX_VIDEO_PIXELS
    };
    let mut scale = (plan.preview_scale * 0.5)
        .min((MAX_DIMENSION - padding - 2.0) / source_width.max(source_height));
    let area_at = |scale: f32| {
        (source_width * scale + padding + 2.0) * (source_height * scale + padding + 2.0)
    };
    if area_at(scale) > max_pixels {
        let mut low = 0.0;
        let mut high = scale;
        for _ in 0..24 {
            let middle = (low + high) / 2.0;
            if area_at(middle) > max_pixels {
                high = middle;
            } else {
                low = middle;
            }
        }
        scale = low;
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(MotionError::invalid("motion layout scale is invalid"));
    }
    let width = (((source_width * scale + padding) / 2.0).ceil() * 2.0).max(32.0);
    let height = (((source_height * scale + padding) / 2.0).ceil() * 2.0).max(32.0);
    Ok(Layout {
        width: (width * pixel_ratio) as u32,
        height: (height * pixel_ratio) as u32,
        scale: scale * pixel_ratio,
        origin_x: ((width - source_width * scale) / 2.0 - bounds.left * scale) * pixel_ratio,
        origin_y: (height - PADDING - bounds.bottom * scale) * pixel_ratio,
    })
}
