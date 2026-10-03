//! Spriteを一度decodeし、再利用PixmapへFrameを直接合成する。

use tiny_skia::{BlendMode, Color, FilterQuality, IntRect, Pixmap, PixmapPaint, Transform};

use super::MotionError;
use super::evaluate::DrawPacket;
use super::layout::Layout;
use super::project::Cut;

struct SpriteCut {
    pixmap: Pixmap,
    normal_bounds: Option<[f32; 4]>,
    light_bounds: Option<[f32; 4]>,
}

pub(super) struct SpriteSheet {
    cuts: Vec<SpriteCut>,
}

impl SpriteSheet {
    pub(super) fn decode(
        data: &[u8],
        cuts: &[Cut],
        context: &crate::media::RenderContext,
    ) -> Result<Self, MotionError> {
        // 圧縮byte数とは別に、展開と切り抜きの合計を512MiB環境向けに制限する。
        let dimensions = data
            .get(16..24)
            .ok_or_else(|| MotionError::invalid("sprite PNG header is missing"))?;
        let width = u32::from_be_bytes(dimensions[..4].try_into().unwrap());
        let height = u32::from_be_bytes(dimensions[4..].try_into().unwrap());
        let cut_pixels = cuts.iter().try_fold(0u64, |sum, cut| {
            let pixels = u64::try_from(cut.width)
                .ok()?
                .checked_mul(u64::try_from(cut.height).ok()?)?;
            sum.checked_add(pixels)
        });
        if u64::from(width) * u64::from(height) > 8 * 1024 * 1024
            || cut_pixels.is_none_or(|pixels| pixels > 16 * 1024 * 1024)
        {
            return Err(MotionError::invalid("sprite memory limit exceeded"));
        }
        context
            .check_memory(
                (u64::from(width) * u64::from(height) + cut_pixels.unwrap_or_default()) * 4,
            )
            .map_err(|error| MotionError::render(error.to_string()))?;
        let sprite = Pixmap::decode_png(data)
            .map_err(|error| MotionError::invalid(format!("sprite PNG decode failed: {error}")))?;
        let mut decoded_cuts = Vec::with_capacity(cuts.len());
        for cut in cuts {
            let x = i32::try_from(cut.x)
                .map_err(|_| MotionError::invalid("sprite cut x is outside the supported range"))?;
            let y = i32::try_from(cut.y)
                .map_err(|_| MotionError::invalid("sprite cut y is outside the supported range"))?;
            let width = u32::try_from(cut.width)
                .map_err(|_| MotionError::invalid("sprite cut width is invalid"))?;
            let height = u32::try_from(cut.height)
                .map_err(|_| MotionError::invalid("sprite cut height is invalid"))?;
            let rect = IntRect::from_xywh(x, y, width, height)
                .ok_or_else(|| MotionError::invalid("sprite cut rectangle is invalid"))?;
            let pixmap = sprite
                .clone_rect(rect)
                .ok_or_else(|| MotionError::invalid("sprite cut is outside the PNG"))?;
            let (normal_bounds, light_bounds) = visible_bounds(&pixmap);
            decoded_cuts.push(SpriteCut {
                pixmap,
                normal_bounds,
                light_bounds,
            });
        }
        Ok(Self { cuts: decoded_cuts })
    }

    pub(super) fn visible_bounds(
        &self,
        cut_index: usize,
        blend_mode: i64,
    ) -> Result<Option<[f32; 4]>, MotionError> {
        let cut = self
            .cuts
            .get(cut_index)
            .ok_or_else(|| MotionError::invalid("draw packet references a missing sprite cut"))?;
        Ok(match blend_mode {
            1 | 3 => cut.light_bounds,
            2 => Some([0.0, 0.0, 1.0, 1.0]),
            _ => cut.normal_bounds,
        })
    }
}

pub(super) struct Rasterizer {
    sprite: SpriteSheet,
    frame: Pixmap,
    layout: Layout,
    multiply_layer: Option<Pixmap>,
}

impl Rasterizer {
    pub(super) fn new(sprite: SpriteSheet, layout: Layout) -> Result<Self, MotionError> {
        let frame = Pixmap::new(layout.width, layout.height)
            .ok_or_else(|| MotionError::render("frame buffer allocation failed"))?;
        Ok(Self {
            sprite,
            frame,
            layout,
            multiply_layer: None,
        })
    }

    pub(super) fn render(&mut self, packets: &[DrawPacket]) -> Result<&[u8], MotionError> {
        self.frame.fill(Color::from_rgba8(0x25, 0x2a, 0x32, 0xff));
        for packet in packets {
            let cut = self.sprite.cuts.get(packet.cut_index).ok_or_else(|| {
                MotionError::invalid("draw packet references a missing sprite cut")
            })?;
            let width = cut.pixmap.width() as f32;
            let height = cut.pixmap.height() as f32;
            let top_left = packet.vertices[0];
            let horizontal = [
                (packet.vertices[3][0] - top_left[0]) * self.layout.scale / width,
                (packet.vertices[3][1] - top_left[1]) * self.layout.scale / width,
            ];
            let vertical = [
                (packet.vertices[1][0] - top_left[0]) * self.layout.scale / height,
                (packet.vertices[1][1] - top_left[1]) * self.layout.scale / height,
            ];
            let transform = Transform::from_row(
                horizontal[0],
                horizontal[1],
                vertical[0],
                vertical[1],
                self.layout.origin_x + top_left[0] * self.layout.scale,
                self.layout.origin_y + top_left[1] * self.layout.scale,
            );
            if !transform.is_finite() {
                return Err(MotionError::invalid("raster transform is not finite"));
            }
            if packet.blend_mode == 2 {
                let layer = self.multiply_layer.get_or_insert_with(|| {
                    Pixmap::new(cut.pixmap.width(), cut.pixmap.height())
                        .expect("validated sprite cut dimensions")
                });
                if layer.width() != cut.pixmap.width() || layer.height() != cut.pixmap.height() {
                    *layer = Pixmap::new(cut.pixmap.width(), cut.pixmap.height())
                        .ok_or_else(|| MotionError::render("multiply layer allocation failed"))?;
                }
                layer.fill(Color::BLACK);
                layer.draw_pixmap(
                    0,
                    0,
                    cut.pixmap.as_ref(),
                    &PixmapPaint {
                        opacity: packet.opacity,
                        blend_mode: BlendMode::SourceOver,
                        quality: FilterQuality::Bilinear,
                    },
                    Transform::identity(),
                    None,
                );
                self.frame.draw_pixmap(
                    0,
                    0,
                    layer.as_ref(),
                    &PixmapPaint {
                        opacity: 1.0,
                        blend_mode: BlendMode::Multiply,
                        quality: FilterQuality::Bilinear,
                    },
                    transform,
                    None,
                );
            } else {
                self.frame.draw_pixmap(
                    0,
                    0,
                    cut.pixmap.as_ref(),
                    &PixmapPaint {
                        opacity: packet.opacity,
                        blend_mode: blend_mode(packet.blend_mode),
                        quality: FilterQuality::Bilinear,
                    },
                    transform,
                    None,
                );
            }
        }
        Ok(self.frame.data())
    }

    pub(super) fn dimensions(&self) -> (u32, u32) {
        (self.layout.width, self.layout.height)
    }

    pub(super) fn rgba(&self) -> &[u8] {
        self.frame.data()
    }
}

fn visible_bounds(pixmap: &Pixmap) -> (Option<[f32; 4]>, Option<[f32; 4]>) {
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let mut normal = PixelBounds::new(width, height);
    let mut light = PixelBounds::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = &pixmap.data()[(y * width + x) * 4..][..4];
            if pixel[3] == 0 {
                continue;
            }
            normal.include(x, y);
            if pixel[..3].iter().any(|value| *value != 0) {
                light.include(x, y);
            }
        }
    }
    (
        normal.normalized(width, height),
        light.normalized(width, height),
    )
}

struct PixelBounds {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl PixelBounds {
    fn new(width: usize, height: usize) -> Self {
        Self {
            left: width,
            top: height,
            right: 0,
            bottom: 0,
        }
    }

    fn include(&mut self, x: usize, y: usize) {
        self.left = self.left.min(x);
        self.top = self.top.min(y);
        self.right = self.right.max(x + 1);
        self.bottom = self.bottom.max(y + 1);
    }

    fn normalized(&self, width: usize, height: usize) -> Option<[f32; 4]> {
        (self.right > self.left && self.bottom > self.top).then_some([
            self.left as f32 / width as f32,
            self.top as f32 / height as f32,
            self.right as f32 / width as f32,
            self.bottom as f32 / height as f32,
        ])
    }
}

fn blend_mode(value: i64) -> BlendMode {
    match value {
        1 => BlendMode::Plus,
        2 => BlendMode::Multiply,
        3 => BlendMode::Screen,
        _ => BlendMode::SourceOver,
    }
}
