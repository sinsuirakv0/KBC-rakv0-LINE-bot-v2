//! `imgcut`、`mamodel`、`maanim`を型付きProjectへ変換する。

use std::collections::{HashMap, HashSet};

use super::MotionError;
use super::assets::MotionAssets;
use super::request::MotionKind;

#[derive(Clone)]
pub(super) struct Cut {
    pub(super) x: i64,
    pub(super) y: i64,
    pub(super) width: i64,
    pub(super) height: i64,
}

#[derive(Clone)]
pub(super) struct ModelPart {
    pub(super) index: usize,
    pub(super) parent: i64,
    pub(super) id: i64,
    pub(super) image_index: i64,
    pub(super) z_index: i64,
    pub(super) x: i64,
    pub(super) y: i64,
    pub(super) pivot_x: i64,
    pub(super) pivot_y: i64,
    pub(super) scale_x: i64,
    pub(super) scale_y: i64,
    pub(super) angle: i64,
    pub(super) opacity: i64,
    pub(super) glow: i64,
}

#[derive(Clone)]
pub(super) struct Model {
    pub(super) parts: Vec<ModelPart>,
    pub(super) scale_unit: i64,
    pub(super) angle_unit: i64,
    pub(super) opacity_unit: i64,
    pub(super) anchor: Option<[i64; 4]>,
}

#[derive(Clone)]
pub(super) struct KeyFrame {
    pub(super) frame: i64,
    pub(super) value: i64,
    pub(super) easing: i64,
    pub(super) easing_power: i64,
}

#[derive(Clone)]
pub(super) struct Track {
    pub(super) part_index: usize,
    pub(super) property: i64,
    pub(super) loop_value: i64,
    pub(super) keys: Vec<KeyFrame>,
    pub(super) first_frame: i64,
    pub(super) last_frame: i64,
    pub(super) offset: i64,
}

pub(super) struct Animation {
    pub(super) tracks: Vec<Track>,
    pub(super) max_frame: i64,
}

pub(super) struct MotionProject {
    pub(super) cuts: Vec<Cut>,
    pub(super) model: Model,
    pub(super) animations: HashMap<MotionKind, Animation>,
}

impl MotionProject {
    pub(super) fn parse(assets: &MotionAssets) -> Result<Self, MotionError> {
        let cuts = parse_imgcut(text(&assets.imgcut, "imgcut")?)?;
        let model = parse_model(text(&assets.model, "mamodel")?)?;
        let animations = assets
            .animations
            .iter()
            .map(|(kind, data)| Ok((*kind, parse_animation(text(data, "maanim")?)?)))
            .collect::<Result<HashMap<_, _>, MotionError>>()?;
        validate_references(&cuts, &model, &animations)?;
        Ok(Self {
            cuts,
            model,
            animations,
        })
    }

    pub(super) fn max_frame(&self, motion: MotionKind) -> Option<u32> {
        self.animations
            .get(&motion)
            .and_then(|animation| u32::try_from(animation.max_frame).ok())
    }
}

fn text<'a>(data: &'a [u8], label: &str) -> Result<&'a str, MotionError> {
    std::str::from_utf8(data)
        .map_err(|error| MotionError::invalid(format!("{label} is not UTF-8: {error}")))
}

fn lines(text: &str) -> Vec<&str> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn integer(line: Option<&&str>, label: &str) -> Result<i64, MotionError> {
    line.copied()
        .unwrap_or_default()
        .parse()
        .map_err(|_| MotionError::invalid(format!("{label} is not an integer")))
}

fn columns(line: Option<&&str>, count: usize, label: &str) -> Result<Vec<i64>, MotionError> {
    let raw = line.copied().unwrap_or_default();
    let values = raw.split(',').collect::<Vec<_>>();
    if values.len() < count {
        return Err(MotionError::invalid(format!(
            "{label} has fewer than {count} columns"
        )));
    }
    values[..count]
        .iter()
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|_| MotionError::invalid(format!("{label} contains a non-integer")))
        })
        .collect()
}

fn header(actual: Option<&&str>, expected: &[&str], label: &str) -> Result<(), MotionError> {
    let actual = actual.copied().unwrap_or_default().to_ascii_lowercase();
    if expected.contains(&actual.as_str()) {
        Ok(())
    } else {
        Err(MotionError::invalid(format!("invalid {label} header")))
    }
}

fn parse_imgcut(text: &str) -> Result<Vec<Cut>, MotionError> {
    let lines = lines(text);
    header(lines.first(), &["[imgcut]"], "imgcut")?;
    let _version = integer(lines.get(1), "imgcut version")?;
    if lines.get(2).is_none_or(|name| name.is_empty()) {
        return Err(MotionError::invalid("imgcut image name is empty"));
    }
    let count = usize::try_from(integer(lines.get(3), "imgcut count")?)
        .map_err(|_| MotionError::invalid("imgcut count is negative"))?;
    if count > 4_096 {
        return Err(MotionError::invalid("imgcut count exceeds the limit"));
    }
    (0..count)
        .map(|index| {
            let values = columns(lines.get(4 + index), 4, "imgcut entry")?;
            if values[2] <= 0 || values[3] <= 0 {
                return Err(MotionError::invalid("imgcut contains an empty rectangle"));
            }
            Ok(Cut {
                x: values[0],
                y: values[1],
                width: values[2],
                height: values[3],
            })
        })
        .collect()
}

fn parse_model(text: &str) -> Result<Model, MotionError> {
    let lines = lines(text);
    header(
        lines.first(),
        &["[modelanim:model]", "[modelanim:model2]", "[mamodel]"],
        "mamodel",
    )?;
    let version = integer(lines.get(1), "mamodel version")?;
    let count = usize::try_from(integer(lines.get(2), "mamodel part count")?)
        .map_err(|_| MotionError::invalid("mamodel part count is negative"))?;
    if count == 0 || count > 4_096 {
        return Err(MotionError::invalid(
            "mamodel part count is outside the limit",
        ));
    }
    let mut cursor = 3;
    let mut parts = Vec::with_capacity(count);
    for index in 0..count {
        let values = columns(lines.get(cursor), 13, "mamodel part")?;
        cursor += 1;
        parts.push(ModelPart {
            index,
            parent: values[0],
            id: values[1],
            image_index: values[2],
            z_index: values[3],
            x: values[4],
            y: values[5],
            pivot_x: values[6],
            pivot_y: values[7],
            scale_x: values[8],
            scale_y: values[9],
            angle: values[10],
            opacity: values[11],
            glow: values[12],
        });
    }
    let (scale_unit, angle_unit, opacity_unit) = if version >= 1 {
        let values = columns(lines.get(cursor), 3, "mamodel units")?;
        cursor += 1;
        (values[0], values[1], values[2])
    } else {
        (100, 360, 255)
    };
    if scale_unit == 0 || angle_unit == 0 || opacity_unit == 0 {
        return Err(MotionError::invalid("mamodel unit cannot be zero"));
    }
    let mut anchor = None;
    if version >= 3 {
        let config_count = usize::try_from(integer(lines.get(cursor), "mamodel config count")?)
            .map_err(|_| MotionError::invalid("mamodel config count is negative"))?;
        cursor += 1;
        if config_count > 1_024 {
            return Err(MotionError::invalid(
                "mamodel config count exceeds the limit",
            ));
        }
        for index in 0..config_count {
            let values = columns(lines.get(cursor + index), 6, "mamodel config")?;
            if index == 0 {
                anchor = Some([values[0], values[1], values[2], values[3]]);
            }
        }
    }
    validate_parents(&parts)?;
    Ok(Model {
        parts,
        scale_unit,
        angle_unit,
        opacity_unit,
        anchor,
    })
}

fn validate_parents(parts: &[ModelPart]) -> Result<(), MotionError> {
    for part in parts {
        if part.parent < -1 || part.parent >= parts.len() as i64 || part.parent == part.index as i64
        {
            return Err(MotionError::invalid(format!(
                "mamodel part {} has an invalid parent",
                part.index
            )));
        }
        let mut visited = HashSet::from([part.index as i64]);
        let mut parent = part.parent;
        while parent >= 0 {
            if !visited.insert(parent) {
                return Err(MotionError::invalid("mamodel parent cycle"));
            }
            parent = parts[parent as usize].parent;
        }
    }
    Ok(())
}

fn parse_animation(text: &str) -> Result<Animation, MotionError> {
    let lines = lines(text);
    header(
        lines.first(),
        &[
            "[modelanim:animation]",
            "[modelanim:animation2]",
            "[maanim]",
        ],
        "maanim",
    )?;
    let _version = integer(lines.get(1), "maanim version")?;
    let count = usize::try_from(integer(lines.get(2), "maanim track count")?)
        .map_err(|_| MotionError::invalid("maanim track count is negative"))?;
    if count > 16_384 {
        return Err(MotionError::invalid("maanim track count exceeds the limit"));
    }
    let mut cursor = 3;
    let mut tracks = Vec::with_capacity(count);
    for _ in 0..count {
        let values = columns(lines.get(cursor), 5, "maanim track")?;
        cursor += 1;
        let key_count = usize::try_from(integer(lines.get(cursor), "maanim key count")?)
            .map_err(|_| MotionError::invalid("maanim key count is negative"))?;
        cursor += 1;
        if key_count > 65_536 {
            return Err(MotionError::invalid("maanim key count exceeds the limit"));
        }
        let mut keys = Vec::with_capacity(key_count);
        for _ in 0..key_count {
            let key = columns(lines.get(cursor), 4, "maanim key")?;
            cursor += 1;
            if !(0..=3).contains(&key[2]) {
                return Err(MotionError::invalid("maanim easing is unsupported"));
            }
            keys.push(KeyFrame {
                frame: key[0],
                value: key[1],
                easing: key[2],
                easing_power: key[3],
            });
        }
        let loop_value = values[2];
        let offset = keys.first().map_or(0, |key| {
            if key.frame < 0 || loop_value != 1 {
                -key.frame
            } else {
                0
            }
        });
        for key in &mut keys {
            key.frame += offset;
        }
        tracks.push(Track {
            part_index: usize::try_from(values[0])
                .map_err(|_| MotionError::invalid("maanim part index is negative"))?,
            property: values[1],
            loop_value,
            first_frame: keys.first().map_or(0, |key| key.frame),
            last_frame: keys.last().map_or(0, |key| key.frame),
            keys,
            offset,
        });
    }
    let max_frame = tracks.iter().map(track_max_frame).max().unwrap_or(1).max(1);
    Ok(Animation { tracks, max_frame })
}

fn track_max_frame(track: &Track) -> i64 {
    if track.keys.is_empty() {
        0
    } else if track.loop_value != -1 {
        if track.loop_value > 1 {
            track.first_frame + (track.last_frame - track.first_frame) * track.loop_value
                - track.offset
        } else {
            track.last_frame - track.offset
        }
    } else {
        track.last_frame - track.offset.min(0)
    }
}

fn validate_references(
    cuts: &[Cut],
    model: &Model,
    animations: &HashMap<MotionKind, Animation>,
) -> Result<(), MotionError> {
    for part in &model.parts {
        if part.image_index >= cuts.len() as i64 {
            return Err(MotionError::invalid("mamodel references a missing cut"));
        }
    }
    for animation in animations.values() {
        for track in &animation.tracks {
            if track.part_index >= model.parts.len() {
                return Err(MotionError::invalid("maanim references a missing part"));
            }
            if track.property == 2
                && track
                    .keys
                    .iter()
                    .any(|key| key.value < 0 || key.value >= cuts.len() as i64)
            {
                return Err(MotionError::invalid("maanim references a missing cut"));
            }
        }
    }
    Ok(())
}
