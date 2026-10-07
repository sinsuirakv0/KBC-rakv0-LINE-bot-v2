//! 旧LINEの時刻条件を共通Modelへ移し、個別指定では全IDを扱う。
use super::time::jst;
use crate::{Result, skd::model::*};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

pub(super) fn periods(
    entry: &SaleEntry,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(i64, i64)>> {
    let start = parse_header_date(&entry.header.start_date, &entry.header.start_time)?;
    let end = parse_header_date(&entry.header.end_date, &entry.header.end_time)?;
    if entry.time_blocks.is_empty() {
        return Ok(vec![(start.timestamp_millis(), end.timestamp_millis())]);
    }
    let mut result = std::collections::BTreeSet::new();
    let mut add = |a: DateTime<Utc>, b: DateTime<Utc>| -> Result<()> {
        if a >= start && a < end && b.min(end) > a {
            result.insert((a.timestamp_millis(), b.min(end).timestamp_millis()));
            if result.len() > 4096 {
                return Err("PushOccurrenceLimit".into());
            }
        }
        Ok(())
    };
    for block in &entry.time_blocks {
        if !block.date_ranges.is_empty() {
            for year in from.with_timezone(&jst()).year() - 1..=to.with_timezone(&jst()).year() + 1
            {
                for range in &block.date_ranges {
                    let annual = |year: i32, text: &str| -> Result<DateTime<Utc>> {
                        let mut parts = text.split_whitespace();
                        let date = format!("{:0>4}", parts.next().ok_or("InvalidAnnualDate")?);
                        let time = parts.next().unwrap_or("0000");
                        if time == "2400" {
                            Ok(parse_header_date(&format!("{year}{date}"), "0000")?
                                + Duration::days(1))
                        } else {
                            parse_header_date(&format!("{year}{date}"), time).map_err(Into::into)
                        }
                    };
                    let a = annual(year, &range.start)?;
                    let mut b = annual(year, &range.end)?;
                    if b <= a {
                        b = annual(year + 1, &range.end)?;
                    }
                    add(a, b)?;
                }
            }
            continue;
        }
        let mut day = from.with_timezone(&jst()).date_naive() - Duration::days(2);
        let last = to.with_timezone(&jst()).date_naive() + Duration::days(1);
        while day <= last {
            let weekday = match day.weekday() {
                chrono::Weekday::Sun => "Sun",
                chrono::Weekday::Mon => "Mon",
                chrono::Weekday::Tue => "Tue",
                chrono::Weekday::Wed => "Wed",
                chrono::Weekday::Thu => "Thu",
                chrono::Weekday::Fri => "Fri",
                chrono::Weekday::Sat => "Sat",
            };
            if (block.month_days.is_empty() || block.month_days.contains(&(day.day() as i64)))
                && (block.weekdays.is_empty() || block.weekdays.iter().any(|w| w == weekday))
            {
                if block.time_ranges.is_empty() {
                    add(point(day, "0000")?, point(day + Duration::days(1), "0000")?)?;
                }
                for range in &block.time_ranges {
                    let a = point(day, &range[0])?;
                    let mut b = point(day, &range[1])?;
                    if b <= a {
                        b += Duration::days(1);
                    }
                    add(a, b)?;
                }
            }
            day += Duration::days(1);
        }
    }
    Ok(result.into_iter().collect())
}
fn point(day: NaiveDate, time: &str) -> Result<DateTime<Utc>> {
    let time = format!("{:0>4}", time.trim());
    let (day, time) = if time == "2400" {
        (day + Duration::days(1), "0000")
    } else {
        (day, time.as_str())
    };
    Ok(parse_header_date(&day.format("%Y%m%d").to_string(), time)?)
}

pub(super) fn all_event(id: i64) -> bool {
    !(14_000..=14_999).contains(&id) && !crate::skd::labels::is_mission_id(id)
}
