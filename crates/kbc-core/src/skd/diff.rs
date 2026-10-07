//! 前後のScheduleを追加分と期間変更へ分類する。

use std::collections::HashSet;

use serde::Serialize;

use super::model::ScheduleHeader;
use super::model::{GachaBlock, GachaHeader, GachaJson};
use super::model::{ItemEntry, ItemJson};
use super::model::{SaleEntry, SaleJson};

pub(super) struct ScheduleChange<T> {
    pub(super) before: T,
    pub(super) after: T,
}

#[derive(Default)]
pub(super) struct ScheduleChanges {
    pub(super) gatya: Vec<ScheduleChange<GachaBlock>>,
    pub(super) sale: Vec<ScheduleChange<SaleEntry>>,
    pub(super) item: Vec<ScheduleChange<ItemEntry>>,
}

pub(super) fn compare_gatya(
    before: GachaJson,
    after: GachaJson,
) -> Result<(GachaJson, Vec<ScheduleChange<GachaBlock>>), String> {
    let flatten = |json: GachaJson| {
        json.data
            .into_iter()
            .flat_map(|block| {
                block.gachas.into_iter().map(move |entry| GachaBlock {
                    header: GachaHeader {
                        gacha_count: 1,
                        ..block.header.clone()
                    },
                    gachas: vec![entry],
                    raw: None,
                })
            })
            .collect::<Vec<_>>()
    };
    let (added, changes) = compare_entries(flatten(before), flatten(after), |block| {
        serialize(&(block.header.gacha_type, &block.gachas))
    })?;
    Ok((
        GachaJson {
            _updated_at: String::new(),
            data: added,
        },
        changes,
    ))
}

pub(super) fn compare_sale(
    before: SaleJson,
    after: SaleJson,
) -> Result<(SaleJson, Vec<ScheduleChange<SaleEntry>>), String> {
    let flatten = |entries: &[SaleEntry]| {
        entries
            .iter()
            .flat_map(|entry| {
                entry.stage_ids.iter().map(move |id| SaleEntry {
                    header: entry.header.clone(),
                    time_blocks: entry.time_blocks.clone(),
                    stage_ids: vec![*id],
                    raw: None,
                })
            })
            .collect::<Vec<_>>()
    };
    let (added, changes) = compare_entries(flatten(&before.data), flatten(&after.data), |entry| {
        serialize(&(&entry.time_blocks, &entry.stage_ids))
    })?;
    let added_keys = added
        .iter()
        .map(serialize)
        .collect::<Result<HashSet<_>, _>>()?;
    let data = after
        .data
        .into_iter()
        .filter_map(|mut entry| {
            entry.stage_ids.retain(|id| {
                let value = SaleEntry {
                    header: entry.header.clone(),
                    time_blocks: entry.time_blocks.clone(),
                    stage_ids: vec![*id],
                    raw: None,
                };
                serialize(&value).is_ok_and(|key| added_keys.contains(&key))
            });
            (!entry.stage_ids.is_empty()).then_some(entry)
        })
        .collect();
    Ok((
        SaleJson {
            _updated_at: String::new(),
            data,
        },
        changes,
    ))
}

pub(super) fn compare_item(
    before: ItemJson,
    after: ItemJson,
) -> Result<(ItemJson, Vec<ScheduleChange<ItemEntry>>), String> {
    let (added, changes) = compare_entries(before.data, after.data, |entry| {
        serialize(&(&entry.gift, &entry.time_blocks))
    })?;
    Ok((
        ItemJson {
            _updated_at: String::new(),
            data: added,
        },
        changes,
    ))
}

fn compare_entries<T, F>(
    before: Vec<T>,
    after: Vec<T>,
    identity: F,
) -> Result<(Vec<T>, Vec<ScheduleChange<T>>), String>
where
    T: Clone + Serialize + HasScheduleHeader,
    F: Fn(&T) -> Result<String, String>,
{
    let previous = before
        .iter()
        .map(serialize)
        .collect::<Result<HashSet<_>, _>>()?;
    let current = after
        .iter()
        .map(serialize)
        .collect::<Result<HashSet<_>, _>>()?;
    let mut removed = before
        .into_iter()
        .filter(|entry| serialize(entry).is_ok_and(|key| !current.contains(&key)))
        .collect::<Vec<_>>();
    let mut added = Vec::new();
    let mut changes = Vec::new();
    for entry in after {
        if serialize(&entry).is_ok_and(|key| previous.contains(&key)) {
            continue;
        }
        let entry_identity = identity(&entry)?;
        let mut matched = None;
        let mut difference = usize::MAX;
        for (index, candidate) in removed.iter().enumerate() {
            if identity(candidate)? != entry_identity {
                continue;
            }
            let score = header_difference(candidate.header(), entry.header());
            if score < difference {
                matched = Some(index);
                difference = score;
            }
        }
        if let Some(index) = matched {
            changes.push(ScheduleChange {
                before: removed.remove(index),
                after: entry,
            });
        } else {
            added.push(entry);
        }
    }
    Ok((added, changes))
}

trait HasScheduleHeader {
    fn header(&self) -> &ScheduleHeader;
}

impl HasScheduleHeader for GachaBlock {
    fn header(&self) -> &ScheduleHeader {
        &self.header.schedule
    }
}

impl HasScheduleHeader for SaleEntry {
    fn header(&self) -> &ScheduleHeader {
        &self.header
    }
}

impl HasScheduleHeader for ItemEntry {
    fn header(&self) -> &ScheduleHeader {
        &self.header
    }
}

fn header_difference(left: &ScheduleHeader, right: &ScheduleHeader) -> usize {
    [
        left.start_date != right.start_date,
        left.start_time != right.start_time,
        left.end_date != right.end_date,
        left.end_time != right.end_time,
        left.min_version != right.min_version,
        left.max_version != right.max_version,
    ]
    .into_iter()
    .filter(|different| *different)
    .count()
}

fn serialize(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("schedule comparison failed: {error}"))
}
