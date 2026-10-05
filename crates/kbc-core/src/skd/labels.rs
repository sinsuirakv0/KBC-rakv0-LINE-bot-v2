use super::model::*;
use crate::messages::{Messages, message};

pub(super) fn series_id(mappings: &MappingMaps, gacha_type: i64, gacha_id: i64) -> Option<i64> {
    mappings
        .get(mode_for_type(gacha_type))
        .get(&gacha_id)
        .copied()
}

pub(super) fn entry_labels(entry: &GachaEntry, messages: &Messages) -> String {
    let mut labels = String::new();
    if entry.guaranteed {
        labels.push_str(message!(messages, "skd.guaranteed"));
    }
    labels.push_str(match entry.flags {
        4 => message!(messages, "skd.step_up"),
        20_600 => message!(messages, "skd.ticket_shard"),
        16_384 => message!(messages, "skd.shard"),
        4_216 => message!(messages, "skd.ticket"),
        _ => "",
    });
    labels
}

pub(super) fn type_tag(gacha_type: i64, messages: &Messages) -> &str {
    match gacha_type {
        4 => message!(messages, "skd.event_tag"),
        0 => message!(messages, "skd.normal_tag"),
        _ => "",
    }
}

pub(super) fn mode_for_type(gacha_type: i64) -> GachaMode {
    match gacha_type {
        1 => GachaMode::Rare,
        4 => GachaMode::Event,
        _ => GachaMode::Normal,
    }
}

pub(super) fn stage_name(id: i64, data: &SaleDisplayData) -> String {
    stage_name_with_breaks(id, data, " ")
}

pub(super) fn stage_name_preserving_breaks(id: i64, data: &SaleDisplayData) -> String {
    stage_name_with_breaks(id, data, "\n")
}

fn stage_name_with_breaks(id: i64, data: &SaleDisplayData, line_break: &str) -> String {
    if is_mission_id(id) {
        let lookup_id = if (15_000..=15_999).contains(&id) {
            id - 15_000
        } else {
            id
        };
        let Some(raw_name) = data.mission_names.get(&lookup_id) else {
            return message!(data.messages, "skd.unknown").to_owned();
        };
        let name = raw_name
            .split([',', '，'])
            .next()
            .unwrap_or(raw_name.as_str());
        return strip_display_markup(name, line_break);
    }
    data.sale_names
        .get(&id)
        .or_else(|| data.all_day_event_names.get(&id))
        .map(|name| strip_display_markup(name, line_break))
        .unwrap_or_else(|| message!(data.messages, "skd.unknown").to_owned())
}

fn strip_display_markup(value: &str, line_break: &str) -> String {
    let with_breaks = replace_breaks(value, line_break);
    let mut result = String::new();
    let mut inside_tag = false;
    for character in with_breaks.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => result.push(character),
            _ => {}
        }
    }
    result.trim().to_owned()
}

fn replace_breaks(value: &str, replacement: &str) -> String {
    let mut result = value.to_owned();
    for pattern in ["<br>", "<BR>", "<br/>", "<BR/>", "<br />", "<BR />"] {
        result = result.replace(pattern, replacement);
    }
    result
}

pub(super) fn is_mission_id(id: i64) -> bool {
    (8_000..=9_999).contains(&id)
        || (15_000..=15_999).contains(&id)
        || (17_000..=17_999).contains(&id)
}

fn representative_stage_id(entry: &SaleEntry, card_setting_ids: &[i64]) -> Option<i64> {
    if entry.stage_ids.len() <= 1 {
        return None;
    }
    card_setting_ids
        .iter()
        .copied()
        .find(|id| entry.stage_ids.contains(id))
}

pub(super) fn list_stage_ids(entry: &SaleEntry, card_setting_ids: &[i64]) -> Vec<i64> {
    representative_stage_id(entry, card_setting_ids)
        .map(|id| vec![id])
        .unwrap_or_else(|| entry.stage_ids.clone())
        .into_iter()
        .filter(|id| !is_mission_id(*id))
        .collect()
}

pub(super) fn schedule_name<'a>(entry: &'a ItemEntry, data: &'a ItemDisplayData) -> &'a str {
    if matches!(entry.gift.gift_type, 301 | 302) {
        return data
            .sale_names
            .get(&entry.gift.gift_type)
            .map(String::as_str)
            .or_else(|| {
                data.item_names
                    .get(&entry.gift.gift_type)
                    .map(|item| item.name.as_str())
            })
            .filter(|name| !name.is_empty())
            .or_else(|| (!entry.gift.title.trim().is_empty()).then_some(entry.gift.title.trim()))
            .unwrap_or(message!(data.messages, "skd.unknown"));
    }
    if !entry.gift.title.trim().is_empty() {
        entry.gift.title.trim()
    } else {
        data.item_names
            .get(&entry.gift.gift_type)
            .map(|item| item.name.as_str())
            .unwrap_or(message!(data.messages, "skd.unknown"))
    }
}
