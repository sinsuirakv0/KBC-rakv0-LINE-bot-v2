//! 保存済みRaw TSVを既存Schedule Modelへ変換する。

use serde_json::Number;

use super::model::{DateRange, ScheduleHeader, TimeBlock, validate_header};
use super::model::{GachaBlock, GachaEntry, GachaHeader, GachaJson, GachaRate};
use super::model::{ItemEntry, ItemGift, ItemJson};
use super::model::{SaleEntry, SaleJson};

const MAX_ROWS: usize = 20_000;
const MAX_TIME_BLOCKS: usize = 128;
const MAX_BLOCK_VALUES: usize = 1_000;
const MAX_GACHAS: usize = 128;

pub(super) fn parse_gatya_tsv(text: &str) -> Result<GachaJson, String> {
    let lines = data_lines(text)?;
    let mut data = Vec::new();
    for line in lines {
        let cells = line.split('\t').collect::<Vec<_>>();
        if cells.len() < 10 {
            continue;
        }
        let gacha_count = parse_usize(cells[9], "gacha count")?;
        if gacha_count > MAX_GACHAS {
            return Err("gacha row has too many entries".to_owned());
        }
        let schedule = parse_header(&cells[..6])?;
        let gacha_type = parse_i64(cells[8], "gacha type")?;
        let mut blocks = Vec::with_capacity(gacha_count);
        let mut offset = 10;
        for _ in 0..gacha_count {
            while cells.get(offset).is_some_and(|value| value.is_empty()) {
                offset += 1;
            }
            let end = offset.saturating_add(15).min(cells.len());
            blocks.push(cells[offset..end].to_vec());
            offset = offset.saturating_add(15);
        }
        while cells
            .get(offset)
            .is_some_and(|value| value.is_empty() || value.parse::<f64>().is_err())
        {
            offset += 1;
        }
        let mut featured_rates = Vec::with_capacity(gacha_count);
        for _ in 0..gacha_count {
            let featured = cells
                .get(offset + 1)
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            featured_rates.push(featured);
            offset += 2;
        }

        let mut gachas = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            let id = block
                .first()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(-1);
            if id == -1 {
                continue;
            }
            let value = |index: usize, label: &str| -> Result<Number, String> {
                parse_number(block.get(index).copied().unwrap_or("0"), label)
            };
            let flags = parse_i64(block.get(3).copied().unwrap_or("0"), "gacha flags")?;
            let guaranteed = parse_i64(
                block.get(11).copied().unwrap_or("0"),
                "gacha guaranteed flag",
            )? == 1;
            let _featured_rate = featured_rates[index];
            gachas.push(GachaEntry {
                id,
                price: value(1, "gacha price")?,
                flags,
                rates: GachaRate {
                    normal: value(4, "normal rate")?,
                    rare: value(6, "rare rate")?,
                    super_rare: value(8, "super rare rate")?,
                    uber_rare: value(10, "uber rare rate")?,
                    legend_rare: value(12, "legend rare rate")?,
                },
                guaranteed,
                message: Some(block.get(14).copied().unwrap_or("").to_owned()),
            });
        }
        data.push(GachaBlock {
            header: GachaHeader {
                schedule,
                gacha_type,
                gacha_count: gacha_count as i64,
            },
            gachas,
        });
    }
    Ok(GachaJson {
        _updated_at: String::new(),
        data,
    })
}

pub(super) fn parse_sale_tsv(text: &str) -> Result<SaleJson, String> {
    let mut data = Vec::new();
    for line in data_lines(text)? {
        let parts = line
            .split('\t')
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 9 {
            return Err("sale row is too short".to_owned());
        }
        let header = parse_header(&parts[..6])?;
        let mut cursor = Cursor::new(&parts[7..]);
        let time_blocks = parse_time_blocks(&mut cursor)?;
        let stage_count = cursor.usize("stage count")?;
        if stage_count > MAX_BLOCK_VALUES {
            return Err("sale row has too many stage IDs".to_owned());
        }
        let mut stage_ids = Vec::with_capacity(stage_count);
        for _ in 0..stage_count {
            stage_ids.push(cursor.i64("stage ID")?);
        }
        data.push(SaleEntry {
            header,
            time_blocks,
            stage_ids,
        });
    }
    Ok(SaleJson {
        _updated_at: String::new(),
        data,
    })
}

pub(super) fn parse_item_tsv(text: &str) -> Result<ItemJson, String> {
    let mut data = Vec::new();
    for line in data_lines(text)? {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 9 {
            return Err("item row is too short".to_owned());
        }
        let header = parse_header(&parts[..6])?;
        let mut cursor = Cursor::new(&parts[7..]);
        let time_blocks = parse_time_blocks(&mut cursor)?;
        let remaining = cursor.remaining();
        let mut gift = ItemGift {
            event_id: 0,
            gift_type: 0,
            gift_amount: 0,
            title: String::new(),
            message: String::new(),
            url: String::new(),
            repeat_flag: 0,
        };
        if remaining.len() >= 8 {
            gift.event_id = parse_i64_or_zero(remaining[0]);
            gift.gift_type = parse_i64_or_zero(remaining[1]);
            gift.gift_amount = parse_i64_or_zero(remaining[2]);
            gift.repeat_flag = parse_i64_or_zero(remaining[7]);
            let raw_title = remaining[3];
            let raw_message_or_url = remaining[4];
            if raw_message_or_url.starts_with("http") {
                gift.title = raw_title.to_owned();
                gift.url = raw_message_or_url.to_owned();
            } else if raw_title.starts_with("http") {
                gift.url = raw_title.to_owned();
            } else {
                gift.title = raw_title.to_owned();
                gift.message = raw_message_or_url.to_owned();
            }
        }
        data.push(ItemEntry {
            header,
            time_blocks,
            gift,
        });
    }
    Ok(ItemJson {
        _updated_at: String::new(),
        data,
    })
}

fn data_lines(text: &str) -> Result<Vec<&str>, String> {
    let lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "[start]" && *line != "[end]")
        .collect::<Vec<_>>();
    if lines.len() > MAX_ROWS {
        return Err("schedule TSV has too many rows".to_owned());
    }
    Ok(lines)
}

fn parse_header(cells: &[&str]) -> Result<ScheduleHeader, String> {
    if cells.len() < 6 {
        return Err("schedule header is incomplete".to_owned());
    }
    let header = ScheduleHeader {
        start_date: cells[0].to_owned(),
        start_time: cells[1].to_owned(),
        end_date: cells[2].to_owned(),
        end_time: cells[3].to_owned(),
        min_version: cells[4].to_owned(),
        max_version: cells[5].to_owned(),
    };
    validate_header(&header)?;
    Ok(header)
}

fn parse_time_blocks(cursor: &mut Cursor<'_>) -> Result<Vec<TimeBlock>, String> {
    let count = cursor.usize("time block count")?;
    if count > MAX_TIME_BLOCKS {
        return Err("schedule row has too many time blocks".to_owned());
    }
    let mut blocks = Vec::with_capacity(count);
    for _ in 0..count {
        let year_count = cursor.usize("date range count")?;
        if year_count > MAX_BLOCK_VALUES {
            return Err("time block has too many date ranges".to_owned());
        }
        let mut date_ranges = Vec::with_capacity(year_count);
        for _ in 0..year_count {
            date_ranges.push(DateRange {
                start: format!(
                    "{} {}",
                    cursor.next("start date")?,
                    cursor.next("start time")?
                ),
                end: format!("{} {}", cursor.next("end date")?, cursor.next("end time")?),
            });
        }
        let month_count = cursor.usize("month day count")?;
        if month_count > MAX_BLOCK_VALUES {
            return Err("time block has too many month days".to_owned());
        }
        let mut month_days = Vec::with_capacity(month_count);
        for _ in 0..month_count {
            month_days.push(cursor.i64("month day")?);
        }
        let weekdays = decode_weekdays(cursor.i64("weekday bitmask")?);
        let time_count = cursor.usize("time range count")?;
        if time_count > MAX_BLOCK_VALUES {
            return Err("time block has too many time ranges".to_owned());
        }
        let mut time_ranges = Vec::with_capacity(time_count);
        for _ in 0..time_count {
            time_ranges.push([
                cursor.next("time range start")?.to_owned(),
                cursor.next("time range end")?.to_owned(),
            ]);
        }
        blocks.push(TimeBlock {
            date_ranges,
            month_days,
            weekdays,
            time_ranges,
        });
    }
    Ok(blocks)
}

fn decode_weekdays(bitmask: i64) -> Vec<String> {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .into_iter()
        .enumerate()
        .filter(|(index, _)| bitmask & (1 << index) != 0)
        .map(|(_, value)| value.to_owned())
        .collect()
}

fn parse_i64(value: &str, label: &str) -> Result<i64, String> {
    value.trim().parse().map_err(|_| format!("invalid {label}"))
}

fn parse_i64_or_zero(value: &str) -> i64 {
    value.trim().parse().unwrap_or(0)
}

fn parse_usize(value: &str, label: &str) -> Result<usize, String> {
    value.trim().parse().map_err(|_| format!("invalid {label}"))
}

fn parse_number(value: &str, label: &str) -> Result<Number, String> {
    value.trim().parse().map_err(|_| format!("invalid {label}"))
}

struct Cursor<'a> {
    cells: &'a [&'a str],
    index: usize,
}

impl<'a> Cursor<'a> {
    fn new(cells: &'a [&'a str]) -> Self {
        Self { cells, index: 0 }
    }

    fn next(&mut self, label: &str) -> Result<&'a str, String> {
        let value = self
            .cells
            .get(self.index)
            .copied()
            .ok_or_else(|| format!("missing {label}"))?;
        self.index += 1;
        Ok(value)
    }

    fn usize(&mut self, label: &str) -> Result<usize, String> {
        parse_usize(self.next(label)?, label)
    }

    fn i64(&mut self, label: &str) -> Result<i64, String> {
        parse_i64(self.next(label)?, label)
    }

    fn remaining(&self) -> &'a [&'a str] {
        &self.cells[self.index..]
    }
}
