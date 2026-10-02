//! 型付きMotion Projectを整数Frameごとの描画Packetへ評価する。

use std::collections::HashSet;
use std::f64::consts::TAU;

use num_bigint::BigInt;

use super::MotionError;
use super::project::{Animation, KeyFrame, Model, ModelPart, MotionProject, Track};
use super::request::MotionKind;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DrawPacket {
    pub(super) cut_index: usize,
    pub(super) part_index: usize,
    pub(super) blend_mode: i64,
    pub(super) opacity: f32,
    pub(super) vertices: [[f32; 2]; 4],
}

#[derive(Clone)]
struct PartState {
    base_index: usize,
    parent: i64,
    id: i64,
    image_index: i64,
    z_index: i64,
    z_order: i64,
    x: f64,
    y: f64,
    pivot_x: f64,
    pivot_y: f64,
    scale_factor_x: f64,
    scale_factor_y: f64,
    opacity_factor: f64,
    angle: f64,
    glow: i64,
    flip_x: i64,
    flip_y: i64,
    native_scale_x: f64,
    native_scale_y: f64,
    native_opacity: f64,
    native_flip_x: bool,
    native_flip_y: bool,
    matrix: Option<[f64; 6]>,
    vertices: Option<[[f64; 2]; 4]>,
}

pub(super) fn evaluate(
    project: &MotionProject,
    motion: MotionKind,
    frame: u32,
) -> Result<Vec<DrawPacket>, MotionError> {
    let animation = project
        .animations
        .get(&motion)
        .ok_or_else(|| MotionError::invalid("requested animation is missing"))?;
    if i64::from(frame) > animation.max_frame {
        return Err(MotionError::invalid(format!(
            "frame {frame} exceeds animation maximum {}",
            animation.max_frame
        )));
    }
    let mut parts = integer_pose(&project.model, animation, i64::from(frame))?;
    calculate_transforms(&mut parts, project)?;
    parts.sort_by_key(|part| part.z_order);
    packets(project, &parts)
}

fn reset_part(base: &ModelPart, model: &Model) -> PartState {
    PartState {
        base_index: base.index,
        parent: base.parent,
        id: base.id,
        image_index: base.image_index,
        z_index: base.z_index,
        z_order: base.z_index * model.parts.len() as i64 + base.index as i64,
        x: base.x as f64,
        y: base.y as f64,
        pivot_x: base.pivot_x as f64,
        pivot_y: base.pivot_y as f64,
        scale_factor_x: model.scale_unit as f64,
        scale_factor_y: model.scale_unit as f64,
        opacity_factor: model.opacity_unit as f64,
        angle: base.angle as f64,
        glow: base.glow,
        flip_x: 1,
        flip_y: 1,
        native_scale_x: 0.0,
        native_scale_y: 0.0,
        native_opacity: 0.0,
        native_flip_x: false,
        native_flip_y: false,
        matrix: None,
        vertices: None,
    }
}

fn integer_pose(
    model: &Model,
    animation: &Animation,
    frame: i64,
) -> Result<Vec<PartState>, MotionError> {
    let mut parts = model
        .parts
        .iter()
        .map(|part| reset_part(part, model))
        .collect::<Vec<_>>();
    for track in &animation.tracks {
        let resolved_frame = resolve_track_frame(track, frame, animation.max_frame);
        if let Some(value) = evaluate_track(track, resolved_frame)? {
            apply_property(&mut parts, track.part_index, track.property, value, model)?;
        }
    }
    validate_animated_parents(&mut parts);
    Ok(parts)
}

fn positive_modulo(value: i64, divisor: i64) -> i64 {
    if divisor == 0 {
        0
    } else {
        ((value % divisor) + divisor) % divisor
    }
}

fn resolve_track_frame(track: &Track, frame: i64, animation_max_frame: i64) -> i64 {
    let loop_length = track.last_frame - track.first_frame;
    let modulus = if track.loop_value == -1 {
        track.last_frame
    } else {
        animation_max_frame + 1
    };
    let mut frame = positive_modulo(frame + track.offset, modulus);
    if track.loop_value > 0 && loop_length != 0 {
        if frame > track.first_frame + track.loop_value * loop_length {
            return track.last_frame;
        }
        if frame > track.first_frame && frame < track.first_frame + track.loop_value * loop_length {
            frame = track.first_frame + positive_modulo(frame - track.first_frame, loop_length);
        } else if frame >= track.first_frame + track.loop_value * loop_length {
            frame = track.last_frame;
        }
    }
    frame
}

fn evaluate_track(track: &Track, frame: i64) -> Result<Option<f64>, MotionError> {
    let keys = &track.keys;
    if keys.is_empty() || frame < keys[0].frame {
        return Ok(None);
    }
    for (index, current) in keys.iter().enumerate() {
        if frame == current.frame {
            return Ok(Some(current.value as f64));
        }
        let Some(next) = keys.get(index + 1) else {
            continue;
        };
        if frame <= current.frame || frame >= next.frame {
            continue;
        }
        if current.easing == 1 {
            return Ok(Some(current.value as f64));
        }
        if current.easing == 3 {
            return lagrange(keys, index, frame).map(Some);
        }
        let numerator = frame - current.frame;
        let denominator = next.frame - current.frame;
        if current.easing == 0 {
            return Ok(Some(
                current.value as f64
                    + divide(
                        ((next.value - current.value) * numerator) as f64,
                        denominator as f64,
                    )?,
            ));
        }
        let progress = numerator as f64 / denominator as f64;
        let power = current.easing_power;
        let eased = if power < 0 {
            (1.0 - (1.0 - progress).powi((-power) as i32)).sqrt()
        } else {
            1.0 - (1.0 - progress.powi(power as i32)).sqrt()
        };
        return Ok(Some(
            (current.value as f64 + (next.value - current.value) as f64 * eased).trunc(),
        ));
    }
    Ok(keys
        .last()
        .filter(|last| frame > last.frame)
        .map(|last| last.value as f64))
}

fn lagrange(keys: &[KeyFrame], index: usize, frame: i64) -> Result<f64, MotionError> {
    let mut low = index;
    let mut high = index;
    while low > 0 && keys[low - 1].easing == 3 {
        low -= 1;
    }
    while high < keys.len() - 1 {
        high += 1;
        if keys[high].easing != 3 {
            break;
        }
    }
    let mut sum = BigInt::from(0);
    for current in low..=high {
        let mut value = BigInt::from(keys[current].value) << 12;
        for other in low..=high {
            if other == current {
                continue;
            }
            let denominator = keys[current].frame - keys[other].frame;
            if denominator == 0 {
                return Err(MotionError::invalid("duplicate Lagrange key frame"));
            }
            value = value * BigInt::from(frame - keys[other].frame) / BigInt::from(denominator);
        }
        sum += value;
    }
    let value = sum / BigInt::from(4096);
    value
        .to_string()
        .parse()
        .map_err(|_| MotionError::invalid("Lagrange result is outside the supported range"))
}

fn divide(numerator: f64, denominator: f64) -> Result<f64, MotionError> {
    if denominator == 0.0 {
        Err(MotionError::invalid("division by zero in motion data"))
    } else {
        Ok((numerator / denominator).trunc())
    }
}

fn apply_property(
    parts: &mut [PartState],
    part_index: usize,
    property: i64,
    value: f64,
    model: &Model,
) -> Result<(), MotionError> {
    let base = &model.parts[part_index];
    if property == 0 {
        let parent = value.trunc() as i64;
        parts[part_index].parent = if valid_parent(part_index, parent, parts) {
            parent
        } else if part_index == 0 {
            -1
        } else {
            0
        };
        return Ok(());
    }
    let part_count = parts.len() as i64;
    let part = &mut parts[part_index];
    match property {
        1 => part.id = value.trunc() as i64,
        2 => part.image_index = value.trunc() as i64,
        3 => {
            part.z_index = value.trunc() as i64;
            part.z_order = part.z_index * part_count + part_index as i64;
        }
        4 => part.x = base.x as f64 + value,
        5 => part.y = base.y as f64 + value,
        6 => part.pivot_x = base.pivot_x as f64 + value,
        7 => part.pivot_y = base.pivot_y as f64 + value,
        8 => {
            part.scale_factor_x = value;
            part.scale_factor_y = value;
        }
        9 => part.scale_factor_x = value,
        10 => part.scale_factor_y = value,
        11 => part.angle = base.angle as f64 + value,
        12 => part.opacity_factor = value,
        13 => part.flip_x = if value == 0.0 { 1 } else { -1 },
        14 => part.flip_y = if value == 0.0 { 1 } else { -1 },
        _ => {}
    }
    Ok(())
}

fn valid_parent(part_index: usize, parent: i64, parts: &[PartState]) -> bool {
    if parent < 0 || parent >= parts.len() as i64 || parent == part_index as i64 {
        return false;
    }
    let mut visited = HashSet::from([part_index as i64]);
    let mut current = parent;
    while current >= 0 {
        if !visited.insert(current) {
            return false;
        }
        current = parts.get(current as usize).map_or(-1, |part| part.parent);
    }
    true
}

fn validate_animated_parents(parts: &mut [PartState]) {
    for index in 0..parts.len() {
        if !valid_parent(index, parts[index].parent, parts) {
            parts[index].parent = if index == 0 { -1 } else { 0 };
        }
    }
}

fn calculate_transforms(
    parts: &mut [PartState],
    project: &MotionProject,
) -> Result<(), MotionError> {
    let mut pending = (0..parts.len()).collect::<Vec<_>>();
    let mut completed = HashSet::from([-1_i64]);
    while !pending.is_empty() {
        let mut progressed = false;
        let mut cursor = 0;
        while cursor < pending.len() {
            let index = pending[cursor];
            if !completed.contains(&parts[index].parent) {
                cursor += 1;
                continue;
            }
            calculate_part_transform(index, parts, project)?;
            pending.remove(cursor);
            completed.insert(index as i64);
            progressed = true;
        }
        if !progressed {
            return Err(MotionError::invalid(
                "animated parent graph cannot be resolved",
            ));
        }
    }
    Ok(())
}

fn calculate_part_transform(
    index: usize,
    parts: &mut [PartState],
    project: &MotionProject,
) -> Result<(), MotionError> {
    let parent = (parts[index].parent >= 0).then(|| parts[parts[index].parent as usize].clone());
    let base = &project.model.parts[index];
    let scale_unit = project.model.scale_unit as f64;
    let opacity_unit = project.model.opacity_unit as f64;
    let part = &mut parts[index];
    if let Some(parent) = parent {
        part.native_scale_x = divide(
            divide(
                base.scale_x as f64 * part.scale_factor_x * parent.native_scale_x,
                scale_unit,
            )?,
            scale_unit,
        )?;
        part.native_scale_y = divide(
            divide(
                base.scale_y as f64 * part.scale_factor_y * parent.native_scale_y,
                scale_unit,
            )?,
            scale_unit,
        )?;
        part.native_opacity = divide(
            divide(
                base.opacity as f64 * part.opacity_factor * parent.native_opacity,
                opacity_unit,
            )?,
            opacity_unit,
        )?;
        part.native_flip_x = (part.flip_x < 0) != parent.native_flip_x;
        part.native_flip_y = (part.flip_y < 0) != parent.native_flip_y;
        part.matrix = Some(translate(
            parent
                .matrix
                .ok_or_else(|| MotionError::invalid("parent transform is missing"))?,
            divide(parent.native_scale_x * part.x, scale_unit)?,
            divide(parent.native_scale_y * part.y, scale_unit)?,
        ));
    } else {
        part.native_scale_x = divide(base.scale_x as f64 * part.scale_factor_x, scale_unit)?;
        part.native_scale_y = divide(base.scale_y as f64 * part.scale_factor_y, scale_unit)?;
        part.native_opacity = divide(base.opacity as f64 * part.opacity_factor, opacity_unit)?;
        part.native_flip_x = part.flip_x < 0;
        part.native_flip_y = part.flip_y < 0;
        part.matrix = Some(translation(part.x, part.y));
    }
    if part.flip_x < 0 {
        part.native_scale_x = -part.native_scale_x;
    }
    if part.flip_y < 0 {
        part.native_scale_y = -part.native_scale_y;
    }
    let mut angle = part.angle * TAU / project.model.angle_unit as f64;
    if part.native_flip_x != part.native_flip_y {
        angle = -angle;
    }
    part.matrix = Some(rotate(part.matrix.unwrap(), angle));
    if part.id < 0 || part.image_index < 0 {
        return Ok(());
    }
    let Some(cut) = project.cuts.get(part.image_index as usize) else {
        return Ok(());
    };
    let left = divide(-part.native_scale_x * part.pivot_x, scale_unit)?;
    let top = divide(-part.native_scale_y * part.pivot_y, scale_unit)?;
    let right = left + divide(part.native_scale_x * cut.width as f64, scale_unit)?;
    let bottom = top + divide(part.native_scale_y * cut.height as f64, scale_unit)?;
    let matrix = part.matrix.unwrap();
    part.vertices = Some([
        point(matrix, left, top),
        point(matrix, left, bottom),
        point(matrix, right, bottom),
        point(matrix, right, top),
    ]);
    Ok(())
}

fn fround(value: f64) -> f64 {
    value as f32 as f64
}

fn translation(x: f64, y: f64) -> [f64; 6] {
    [1.0, 0.0, fround(x), 0.0, 1.0, fround(y)]
}

fn translate(matrix: [f64; 6], x: f64, y: f64) -> [f64; 6] {
    let mut result = matrix;
    result[2] = fround(matrix[2] + fround(matrix[1] * y) + fround(matrix[0] * x));
    result[5] = fround(matrix[5] + fround(matrix[4] * y) + fround(matrix[3] * x));
    result
}

fn rotate(matrix: [f64; 6], angle: f64) -> [f64; 6] {
    if angle == 0.0 {
        return matrix;
    }
    let sine = fround(fround(angle).sin());
    let cosine = fround(fround(angle).cos());
    let [a, b, tx, c, d, ty] = matrix;
    [
        fround(fround(sine * b) + fround(a * cosine)),
        fround(fround(cosine * b) - fround(a * sine)),
        tx,
        fround(fround(sine * d) + fround(c * cosine)),
        fround(fround(cosine * d) - fround(c * sine)),
        ty,
    ]
}

fn point(matrix: [f64; 6], x: f64, y: f64) -> [f64; 2] {
    [
        fround(matrix[2] + fround(matrix[1] * y) + fround(matrix[0] * x)).trunc(),
        fround(matrix[5] + fround(matrix[4] * y) + fround(matrix[3] * x)).trunc(),
    ]
}

fn model_anchor(parts: &[PartState], model: &Model) -> Result<[f64; 2], MotionError> {
    let Some(anchor) = model.anchor else {
        return Ok([0.0, 0.0]);
    };
    let target_index = usize::try_from(anchor[0]).unwrap_or(0);
    let Some(target) = parts.iter().find(|part| part.base_index == target_index) else {
        return Ok([0.0, 0.0]);
    };
    let Some(matrix) = target.matrix else {
        return Ok([0.0, 0.0]);
    };
    let local_x = divide(
        (anchor[2] as f64 - target.pivot_x) * target.native_scale_x,
        model.scale_unit as f64,
    )?;
    let local_y = divide(
        (anchor[3] as f64 - target.pivot_y) * target.native_scale_y,
        model.scale_unit as f64,
    )?;
    Ok(point(matrix, local_x, local_y))
}

fn packets(project: &MotionProject, parts: &[PartState]) -> Result<Vec<DrawPacket>, MotionError> {
    let anchor = model_anchor(parts, &project.model)?;
    let mut blend_mode = 0;
    let mut packets = Vec::new();
    for part in parts {
        let Some(vertices) = part.vertices else {
            continue;
        };
        if (0..4).contains(&part.glow) {
            blend_mode = part.glow;
        }
        let opacity = divide(
            part.native_opacity * 255.0,
            project.model.opacity_unit as f64,
        )?;
        if opacity <= 0.0 {
            continue;
        }
        packets.push(DrawPacket {
            cut_index: part.image_index as usize,
            part_index: part.base_index,
            blend_mode,
            opacity: (opacity / 255.0).clamp(0.0, 1.0) as f32,
            vertices: vertices.map(|vertex| {
                [
                    (vertex[0] - anchor[0]) as f32,
                    (vertex[1] - anchor[1]) as f32,
                ]
            }),
        });
    }
    Ok(packets)
}
