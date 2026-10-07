use crate::skd::model::format_jst_full;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone};
use std::sync::LazyLock;

pub(super) fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

pub(super) fn format_time(value: i64) -> String {
    DateTime::from_timestamp_millis(value)
        .map(format_jst_full)
        .unwrap_or_default()
}

// 旧版の分後・日付・日付と時刻の組合せを維持する。本文は省略できる。
pub(super) fn parse(args: &[String], now: i64) -> Result<(i64, String), &'static str> {
    let first = args.first().ok_or("push.invalid_time")?;
    let now_date = DateTime::from_timestamp_millis(now).ok_or("push.invalid_time")?;
    let (due, used) = if first.bytes().all(|c| c.is_ascii_digit()) && !first.is_empty() {
        let minutes: i64 = first.parse().map_err(|_| "push.invalid_time")?;
        if minutes <= 0 {
            return Err("push.invalid_time");
        }
        (
            now.checked_add(minutes.checked_mul(60_000).ok_or("push.too_far")?)
                .ok_or("push.too_far")?,
            1,
        )
    } else {
        static DATE: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"^(?:(\d{2,4})/)?(\d{1,2})/(\d{1,2})(?:[-(（][-－ー]?(\d{1,2})[:：](\d{2})[)）]?)?$").unwrap()
        });
        static TIME: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"^[-－ー]?(\d{1,2})[:：](\d{2})$").unwrap());
        if first.contains('(') != first.ends_with(')')
            || first.contains('（') != first.ends_with('）')
        {
            return Err("push.invalid_time");
        }
        let date = DATE.captures(first).ok_or("push.invalid_time")?;
        let number = |index: usize| date.get(index).and_then(|v| v.as_str().parse::<u32>().ok());
        let year = number(1).map_or(now_date.with_timezone(&jst()).year(), |year| {
            if year < 100 {
                2000 + year as i32
            } else {
                year as i32
            }
        });
        let mut hour = number(4).unwrap_or(0);
        let mut minute = number(5).unwrap_or(0);
        let mut used = 1;
        if date.get(4).is_none()
            && let Some(time) = args.get(1).and_then(|value| TIME.captures(value))
        {
            hour = time[1].parse().map_err(|_| "push.invalid_time")?;
            minute = time[2].parse().map_err(|_| "push.invalid_time")?;
            used = 2;
        }
        let local = NaiveDate::from_ymd_opt(year, number(2).unwrap_or(0), number(3).unwrap_or(0))
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .ok_or("push.invalid_time")?;
        let due = jst()
            .from_local_datetime(&local)
            .single()
            .ok_or("push.invalid_time")?
            .timestamp_millis();
        (due, used)
    };
    if due <= now {
        return Err("push.past");
    }
    if due - now > 10 * 365 * 24 * 60 * 60_000 {
        return Err("push.too_far");
    }
    Ok((due, args[used..].join(" ")))
}
