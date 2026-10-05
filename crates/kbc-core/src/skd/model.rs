//! Event data共通の日時Modelと表示処理を定義する。

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScheduleHeader {
    pub(super) start_date: String,
    pub(super) start_time: String,
    pub(super) end_date: String,
    pub(super) end_time: String,
    pub(super) min_version: String,
    pub(super) max_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct DateRange {
    pub(super) start: String,
    pub(super) end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TimeBlock {
    pub(super) date_ranges: Vec<DateRange>,
    pub(super) month_days: Vec<i64>,
    pub(super) weekdays: Vec<String>,
    pub(super) time_ranges: Vec<[String; 2]>,
}

pub(super) fn parse_header_date(date_text: &str, time_text: &str) -> Result<DateTime<Utc>, String> {
    let date = format!("{:0>8}", date_text.trim());
    let time = format!("{:0>4}", time_text.trim());
    if date.len() != 8 || time.len() != 4 {
        return Err("date or time has an invalid length".to_owned());
    }
    let year = parse_part(&date, 0, 4)?;
    let month = parse_part(&date, 4, 6)? as u32;
    let day = parse_part(&date, 6, 8)? as u32;
    let hour = parse_part(&time, 0, 2)? as u32;
    let minute = parse_part(&time, 2, 4)? as u32;
    let local_date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| "date is outside the calendar".to_owned())?;
    let local_time = NaiveTime::from_hms_opt(hour, minute, 0)
        .ok_or_else(|| "time is outside the clock".to_owned())?;
    let timezone =
        FixedOffset::east_opt(9 * 60 * 60).ok_or_else(|| "JST offset is invalid".to_owned())?;
    timezone
        .from_local_datetime(&local_date.and_time(local_time))
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| "date and time are ambiguous".to_owned())
}

pub(super) fn validate_header(header: &ScheduleHeader) -> Result<(), String> {
    parse_header_date(&header.start_date, &header.start_time)?;
    parse_header_date(&header.end_date, &header.end_time)?;
    Ok(())
}

pub(super) fn format_jst_short(date: DateTime<Utc>) -> String {
    let value = date.with_timezone(&jst());
    format!(
        "{}/{}({}) {:02}:{:02}",
        value.month(),
        value.day(),
        weekday_japanese(value.weekday()),
        value.hour(),
        value.minute()
    )
}

pub(super) fn format_duration(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let mut remaining_minutes = (end - start).num_minutes().max(0);
    let days = remaining_minutes / 1_440;
    remaining_minutes %= 1_440;
    let hours = remaining_minutes / 60;
    let minutes = remaining_minutes % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    format!(
        "<{}>",
        if parts.is_empty() {
            "0m".to_owned()
        } else {
            parts.concat()
        }
    )
}

fn parse_part(value: &str, start: usize, end: usize) -> Result<i32, String> {
    value
        .get(start..end)
        .ok_or_else(|| "date or time is not ASCII".to_owned())?
        .parse()
        .map_err(|_| "date or time contains a non-number".to_owned())
}

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 60 * 60).expect("JST is a valid fixed offset")
}

fn weekday_japanese(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Sun => "日",
        chrono::Weekday::Mon => "月",
        chrono::Weekday::Tue => "火",
        chrono::Weekday::Wed => "水",
        chrono::Weekday::Thu => "木",
        chrono::Weekday::Fri => "金",
        chrono::Weekday::Sat => "土",
    }
}

use serde_json::Number;
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum GachaMode {
    Rare,
    Event,
    Normal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GachaRate {
    pub(super) normal: Number,
    pub(super) rare: Number,
    pub(super) super_rare: Number,
    pub(super) uber_rare: Number,
    pub(super) legend_rare: Number,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GachaEntry {
    pub(super) id: i64,
    pub(super) price: Number,
    pub(super) flags: i64,
    pub(super) rates: GachaRate,
    pub(super) guaranteed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GachaHeader {
    #[serde(flatten)]
    pub(super) schedule: ScheduleHeader,
    pub(super) gacha_type: i64,
    pub(super) gacha_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GachaBlock {
    pub(super) header: GachaHeader,
    pub(super) gachas: Vec<GachaEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GachaJson {
    #[serde(default, rename = "updatedAt")]
    pub(super) _updated_at: String,
    pub(super) data: Vec<GachaBlock>,
}

pub(super) struct ModeMaps<T> {
    pub(super) rare: T,
    pub(super) event: T,
    pub(super) normal: T,
}

impl<T> ModeMaps<T> {
    pub(super) fn get(&self, mode: GachaMode) -> &T {
        match mode {
            GachaMode::Rare => &self.rare,
            GachaMode::Event => &self.event,
            GachaMode::Normal => &self.normal,
        }
    }
}

pub(super) type NameMaps = ModeMaps<HashMap<i64, String>>;
pub(super) type MappingMaps = ModeMaps<HashMap<i64, i64>>;

pub(super) struct GachaScheduleData {
    pub gacha: GachaJson,
    pub short_series_names: NameMaps,
    pub series_mappings: MappingMaps,
    pub messages: std::sync::Arc<crate::messages::Messages>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SaleEntry {
    pub(super) header: ScheduleHeader,
    pub(super) time_blocks: Vec<TimeBlock>,
    pub(super) stage_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SaleJson {
    #[serde(default, rename = "updatedAt")]
    pub(super) _updated_at: String,
    pub(super) data: Vec<SaleEntry>,
}

pub(super) type OrderedNames = HashMap<i64, String>;
pub(super) struct SaleDisplayData {
    pub sale: SaleJson,
    pub sale_names: OrderedNames,
    pub all_day_event_names: OrderedNames,
    pub mission_names: OrderedNames,
    pub card_setting_stage_ids: Vec<i64>,
    pub messages: std::sync::Arc<crate::messages::Messages>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ItemGift {
    pub(super) event_id: i64,
    pub(super) gift_type: i64,
    pub(super) gift_amount: i64,
    pub(super) title: String,
    pub(super) message: String,
    pub(super) url: String,
    pub(super) repeat_flag: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ItemEntry {
    pub(super) header: ScheduleHeader,
    pub(super) time_blocks: Vec<TimeBlock>,
    pub(super) gift: ItemGift,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ItemJson {
    #[serde(default, rename = "updatedAt")]
    pub(super) _updated_at: String,
    pub(super) data: Vec<ItemEntry>,
}

pub(super) struct ItemName {
    pub(super) name: String,
}

pub(super) struct ItemDisplayData {
    pub messages: std::sync::Arc<crate::messages::Messages>,
    pub(super) item: ItemJson,
    pub(super) item_names: HashMap<i64, ItemName>,
    pub(super) sale_names: HashMap<i64, String>,
}
