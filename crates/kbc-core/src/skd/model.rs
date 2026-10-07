//! Event data共通の日時Modelと表示処理を定義する。

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduleHeader {
    pub(crate) start_date: String,
    pub(crate) start_time: String,
    pub(crate) end_date: String,
    pub(crate) end_time: String,
    pub(crate) min_version: String,
    pub(crate) max_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DateRange {
    pub(crate) start: String,
    pub(crate) end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeBlock {
    pub(crate) date_ranges: Vec<DateRange>,
    pub(crate) month_days: Vec<i64>,
    pub(crate) weekdays: Vec<String>,
    pub(crate) time_ranges: Vec<[String; 2]>,
}

pub(crate) fn parse_header_date(date_text: &str, time_text: &str) -> Result<DateTime<Utc>, String> {
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

pub(crate) fn validate_header(header: &ScheduleHeader) -> Result<(), String> {
    parse_header_date(&header.start_date, &header.start_time)?;
    parse_header_date(&header.end_date, &header.end_time)?;
    Ok(())
}

pub(crate) fn format_jst_short(date: DateTime<Utc>) -> String {
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

pub(crate) fn format_duration(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
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
pub(crate) enum GachaMode {
    Rare,
    Event,
    Normal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GachaRate {
    pub(crate) normal: Number,
    pub(crate) rare: Number,
    pub(crate) super_rare: Number,
    pub(crate) uber_rare: Number,
    pub(crate) legend_rare: Number,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GachaEntry {
    pub(crate) id: i64,
    pub(crate) price: Number,
    pub(crate) flags: i64,
    pub(crate) rates: GachaRate,
    pub(crate) guaranteed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GachaHeader {
    #[serde(flatten)]
    pub(crate) schedule: ScheduleHeader,
    pub(crate) gacha_type: i64,
    pub(crate) gacha_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GachaBlock {
    pub(crate) header: GachaHeader,
    pub(crate) gachas: Vec<GachaEntry>,
    #[serde(default, skip_serializing)]
    pub(crate) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GachaJson {
    #[serde(default, rename = "updatedAt")]
    pub(crate) _updated_at: String,
    pub(crate) data: Vec<GachaBlock>,
}

pub(crate) struct ModeMaps<T> {
    pub(crate) rare: T,
    pub(crate) event: T,
    pub(crate) normal: T,
}

impl<T> ModeMaps<T> {
    pub(crate) fn get(&self, mode: GachaMode) -> &T {
        match mode {
            GachaMode::Rare => &self.rare,
            GachaMode::Event => &self.event,
            GachaMode::Normal => &self.normal,
        }
    }
}

pub(crate) type NameMaps = ModeMaps<HashMap<i64, String>>;
pub(crate) type MappingMaps = ModeMaps<HashMap<i64, i64>>;

pub(crate) struct GachaScheduleData {
    pub gacha: GachaJson,
    pub gacha_names: NameMaps,
    pub short_series_names: NameMaps,
    pub series_mappings: MappingMaps,
    pub messages: std::sync::Arc<crate::messages::Messages>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaleEntry {
    pub(crate) header: ScheduleHeader,
    pub(crate) time_blocks: Vec<TimeBlock>,
    pub(crate) stage_ids: Vec<i64>,
    #[serde(default, skip_serializing)]
    pub(crate) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaleJson {
    #[serde(default, rename = "updatedAt")]
    pub(crate) _updated_at: String,
    pub(crate) data: Vec<SaleEntry>,
}

pub(crate) type OrderedNames = HashMap<i64, String>;
pub(crate) struct SaleDisplayData {
    pub sale: SaleJson,
    pub sale_names: OrderedNames,
    pub all_day_event_names: OrderedNames,
    pub mission_names: OrderedNames,
    pub card_setting_stage_ids: Vec<i64>,
    pub messages: std::sync::Arc<crate::messages::Messages>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemGift {
    pub(crate) event_id: i64,
    pub(crate) gift_type: i64,
    pub(crate) gift_amount: i64,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) url: String,
    pub(crate) repeat_flag: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemEntry {
    pub(crate) header: ScheduleHeader,
    pub(crate) time_blocks: Vec<TimeBlock>,
    pub(crate) gift: ItemGift,
    #[serde(default, skip_serializing)]
    pub(crate) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemJson {
    #[serde(default, rename = "updatedAt")]
    pub(crate) _updated_at: String,
    pub(crate) data: Vec<ItemEntry>,
}

pub(crate) struct ItemName {
    pub(crate) name: String,
    pub(crate) detail: String,
}

pub(crate) struct ItemDisplayData {
    pub messages: std::sync::Arc<crate::messages::Messages>,
    pub(crate) item: ItemJson,
    pub(crate) item_names: HashMap<i64, ItemName>,
    pub(crate) sale_names: HashMap<i64, String>,
}

pub(crate) fn format_jst_full(date: DateTime<Utc>) -> String {
    let value = date.with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap());
    format!(
        "{}年{}月{}日({}) {:02}:{:02}",
        value.year(),
        value.month(),
        value.day(),
        weekday_japanese(value.weekday()),
        value.hour(),
        value.minute()
    )
}
