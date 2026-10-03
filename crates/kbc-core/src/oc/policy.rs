use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UrlRule {
    pub scope: String,
    pub url: String,
}
impl UrlRule {
    pub fn new(raw: &str, scope: &str) -> Option<Self> {
        let url = parse_url(raw)?;
        if url.as_str().len() > 1024 {
            return None;
        }
        (url.scheme() == "https"
            && matches!(scope, "exact" | "path" | "prefix" | "domain")
            && raw.len() <= 1024)
            .then(|| Self {
                scope: scope.into(),
                url: url.to_string(),
            })
    }
    pub fn matches(&self, url: &Url) -> bool {
        let Ok(rule) = Url::parse(&self.url) else {
            return false;
        };
        if url.scheme() != "https" || url.origin() != rule.origin() {
            return false;
        }
        match self.scope.as_str() {
            "domain" => true,
            "path" => url.path() == rule.path(),
            "prefix" => {
                rule.path() == "/"
                    || url.path() == rule.path().trim_end_matches('/')
                    || url
                        .path()
                        .starts_with(&format!("{}/", rule.path().trim_end_matches('/')))
            }
            _ => {
                url.path() == rule.path()
                    && url.query() == rule.query()
                    && url.fragment() == rule.fragment()
            }
        }
    }
}
pub fn parse_url(raw: &str) -> Option<Url> {
    let raw = raw.trim().trim_end_matches([
        '.', ',', ';', ':', '!', '?', '。', '、', '）', '」', '』', '】', ']', '}', '>',
    ]);
    Url::parse(&if raw.contains("://") {
        raw.into()
    } else {
        format!("https://{raw}")
    })
    .ok()
    .filter(|url| url.host_str().is_some())
}
pub fn url_text(text: &str, command: bool) -> &str {
    let trimmed = text.trim_start();
    if command && (trimmed.starts_with('!') || trimmed.starts_with("o.")) {
        trimmed
            .split_once(char::is_whitespace)
            .map_or("", |(_, tail)| tail)
    } else {
        text
    }
}
pub fn urls(text: &str) -> Vec<Url> {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(||regex::Regex::new(r#"(?i)(?:[a-z][a-z0-9+.-]{1,31}://|www\.)[^\s<>"'`]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}(?::[0-9]{2,5})?(?:/[^\s<>"'`]*)?"#).expect("static URL pattern"));
    let text: String = text.nfkc().collect();
    let mut found = Vec::new();
    for hit in pattern.find_iter(&text) {
        if !hit.as_str().contains("://")
            && text[..hit.start()]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || "_@-".contains(c))
        {
            continue;
        }
        if let Some(url) = parse_url(hit.as_str())
            && !found.contains(&url)
        {
            found.push(url);
        }
        if found.len() == 65 {
            break;
        }
    }
    found
}
pub fn danger_word(text: &str) -> Option<&'static str> {
    let normalized: String = text.nfkc().collect();
    ["チート", "代行"]
        .into_iter()
        .find(|word| normalized.contains(word))
}
pub fn role_rank(role: &str) -> u8 {
    match role {
        "ADMIN" | "1" => 3,
        "CO_ADMIN" | "2" => 2,
        "MEMBER" | "10" => 1,
        _ => 0,
    }
}

// 数字は分、時刻だけは次の同時刻、日付は日本時間。旧版の引数も保持する。
pub fn mute_until(raw: &str, now: i64) -> Option<Option<i64>> {
    use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone};
    let value = raw.nfkc().collect::<String>().to_ascii_lowercase();
    if matches!(value.as_str(), "inf" | "infinity" | "forever") {
        return Some(None);
    }
    if let Ok(minutes) = value.parse::<i64>() {
        return (minutes > 0)
            .then_some(minutes)
            .and_then(|m| m.checked_mul(60000))
            .and_then(|n| now.checked_add(n))
            .map(Some);
    }
    let zone = FixedOffset::east_opt(9 * 3600)?;
    let current = zone.timestamp_millis_opt(now).single()?;
    let make = |year, month, day, hour, minute| {
        zone.from_local_datetime(
            &NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)?,
        )
        .single()
        .map(|date| date.timestamp_millis())
    };
    let (date, time) = value
        .split_once('-')
        .map_or((None, value.as_str()), |(date, time)| (Some(date), time));
    if date.is_none() && !time.contains('/') {
        let (hour, minute) = time.split_once(':')?;
        let hour = hour.parse().ok()?;
        let minute = minute.parse().ok()?;
        let at = make(current.year(), current.month(), current.day(), hour, minute)?;
        return Some(Some(if at <= now {
            at.checked_add(86400000)?
        } else {
            at
        }));
    }
    let parts: Vec<&str> = date.unwrap_or(time).split('/').collect();
    let (year, month, day) = match parts.as_slice() {
        [month, day] => (current.year(), month.parse().ok()?, day.parse().ok()?),
        [year, month, day] => (year.parse().ok()?, month.parse().ok()?, day.parse().ok()?),
        _ => return None,
    };
    let (hour, minute) = if date.is_some() {
        let (h, m) = time.split_once(':')?;
        (h.parse().ok()?, m.parse().ok()?)
    } else {
        (0, 0)
    };
    let at = make(year, month, day, hour, minute)?;
    (at > now).then_some(Some(at))
}
