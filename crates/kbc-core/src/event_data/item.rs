//! Discord版の引数解釈と表示をLINEの共通出力へ移植する。
use super::format_time_block;
use super::{Source, push_message};
use crate::skd::labels::schedule_name;
use crate::skd::model::*;
use crate::{
    Result,
    messages::{Messages, message},
};
use chrono::{DateTime, Utc};
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub(super) async fn run(
    source: &Source,
    messages: &Messages,
    arguments: &[String],
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let formatter = Formatter { messages };
    match parse_request(arguments) {
        ItemRequest::Usage => formatter.single_message(message!(messages, "event.item_usage")),
        ItemRequest::Schedule => formatter.single_message(
            formatter
                .format_schedule(&source.item().await?, now)
                .unwrap_or_else(|| message!(messages, "event.no_item").into()),
        ),
        ItemRequest::Detail { id } => formatter.format_details(id, &source.item().await?),
        ItemRequest::Json { id } => {
            formatter.format_json_or_raw(id, false, &source.fetch_item_json().await?)
        }
        ItemRequest::Raw { id } => {
            formatter.format_json_or_raw(id, true, &source.fetch_item_json().await?)
        }
    }
}

enum ItemRequest {
    Schedule,
    Detail { id: i64 },
    Json { id: i64 },
    Raw { id: i64 },
    Usage,
}

fn parse_request(arguments: &[String]) -> ItemRequest {
    if arguments.is_empty() {
        return ItemRequest::Schedule;
    }
    let id = arguments.first().and_then(|value| parse_id(value));
    if arguments.len() == 1 {
        return id.map_or(ItemRequest::Usage, |id| ItemRequest::Detail { id });
    }
    if arguments.len() == 2 {
        let Some(id) = id else {
            return ItemRequest::Usage;
        };
        if arguments[1].eq_ignore_ascii_case("j") || arguments[1].eq_ignore_ascii_case("json") {
            return ItemRequest::Json { id };
        }
        if arguments[1].eq_ignore_ascii_case("r") || arguments[1].eq_ignore_ascii_case("raw") {
            return ItemRequest::Raw { id };
        }
    }
    ItemRequest::Usage
}

fn parse_id(value: &str) -> Option<i64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = value.parse().ok()?;
    (id <= MAX_SAFE_INTEGER).then_some(id)
}

fn search_entries(id: i64, item: &ItemJson) -> (Vec<&ItemEntry>, bool) {
    let by_gift_type = item
        .data
        .iter()
        .filter(|entry| entry.gift.gift_type == id)
        .collect::<Vec<_>>();
    if !by_gift_type.is_empty() {
        return (by_gift_type, false);
    }
    (
        item.data
            .iter()
            .filter(|entry| entry.gift.event_id == id)
            .collect(),
        true,
    )
}

fn replace_html_breaks(value: &str) -> String {
    let mut result = value.to_owned();
    for pattern in ["<br>", "<BR>", "<br/>", "<BR/>", "<br />", "<BR />"] {
        result = result.replace(pattern, "\n");
    }
    result
}

fn decode_html_entities(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find('&') {
        result.push_str(&remainder[..start]);
        let entity_start = &remainder[start..];
        let Some(end) = entity_start.find(';') else {
            result.push_str(entity_start);
            return result;
        };
        let code = &entity_start[1..end];
        let decoded = match code.to_ascii_lowercase().as_str() {
            "amp" => Some('&'),
            "apos" => Some('\''),
            "gt" => Some('>'),
            "lt" => Some('<'),
            "nbsp" => Some(' '),
            "quot" => Some('"'),
            _ if code.starts_with("#x") || code.starts_with("#X") => {
                u32::from_str_radix(&code[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if code.starts_with('#') => code[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(character) = decoded {
            result.push(character);
        } else {
            result.push_str(&entity_start[..=end]);
        }
        remainder = &entity_start[end + 1..];
    }
    result.push_str(remainder);
    result
}

fn header_start(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("item headers are validated when loaded")
}

fn header_end(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("item headers are validated when loaded")
}

fn is_permanent(entry: &ItemEntry) -> bool {
    entry.header.end_date == "20300101"
}

struct Formatter<'a> {
    messages: &'a Messages,
}
impl Formatter<'_> {
    fn format_details(&self, id: i64, data: &ItemDisplayData) -> Result<Vec<String>> {
        let (entries, searched_by_event_id) = search_entries(id, &data.item);
        if entries.is_empty() {
            return self.single_message(self.not_found_message(id));
        }

        let mut output = Vec::new();
        if searched_by_event_id {
            push_message(
                &mut output,
                message!(self.messages, "event.item_event_id", id = id),
            )?;
        }
        for entry in entries {
            push_message(&mut output, self.format_detail(entry, data))?;
        }
        Ok(output)
    }

    fn format_json_or_raw(&self, id: i64, is_raw: bool, item: &ItemJson) -> Result<Vec<String>> {
        let (entries, searched_by_event_id) = search_entries(id, item);
        if entries.is_empty() {
            return self.single_message(self.not_found_message(id));
        }

        let mut output = Vec::new();
        if searched_by_event_id {
            push_message(
                &mut output,
                message!(self.messages, "event.item_event_id", id = id),
            )?;
        }
        for entry in entries {
            if is_raw {
                match &entry.raw {
                    Some(raw) => push_message(&mut output, raw.replace('\t', "    "))?,
                    None => push_message(
                        &mut output,
                        message!(
                            self.messages,
                            "event.raw_missing",
                            arg0 = entry.header.start_date
                        ),
                    )?,
                }
            } else {
                push_message(&mut output, serde_json::to_string_pretty(entry)?)?;
            }
        }
        Ok(output)
    }

    fn format_detail(&self, entry: &ItemEntry, data: &ItemDisplayData) -> String {
        let start = header_start(entry);
        let end_text = if is_permanent(entry) {
            message!(self.messages, "skd.permanent").to_owned()
        } else {
            format_jst_full(header_end(entry))
        };
        let amount = if entry.gift.gift_amount > 0 {
            message!(
                self.messages,
                "event.gift_amount",
                arg0 = entry.gift.gift_amount
            )
        } else {
            String::new()
        };
        let mut lines = vec![
            data.item_names
                .get(&entry.gift.gift_type)
                .map(|item| item.name.as_str())
                .unwrap_or(message!(self.messages, "skd.unknown"))
                .to_owned(),
            format!(
                "{} ~ {}  ver.{}~{}",
                format_jst_full(start),
                end_text,
                entry.header.min_version,
                entry.header.max_version
            ),
            format!("eventId: {}", entry.gift.event_id),
            format!("giftType: {}{amount}", entry.gift.gift_type),
        ];
        if entry.gift.repeat_flag == 0 {
            lines.push(message!(self.messages, "event.once_only").to_owned());
        }

        let gift_detail = data
            .item_names
            .get(&entry.gift.gift_type)
            .map(|item| self.format_gift_detail(&item.detail))
            .unwrap_or_default();
        if !gift_detail.is_empty() {
            lines.extend([
                String::new(),
                message!(self.messages, "event.gift_details").to_owned(),
                gift_detail,
            ]);
        }

        let mut extras = Vec::new();
        if !entry.gift.title.is_empty() {
            extras.push(entry.gift.title.clone());
        }
        if !entry.gift.message.is_empty() {
            extras.push(replace_html_breaks(&entry.gift.message));
        }
        if !entry.gift.url.is_empty() {
            extras.push(entry.gift.url.clone());
        }
        if !extras.is_empty() {
            lines.push(String::new());
            lines.extend(extras);
        }
        if !entry.time_blocks.is_empty() {
            lines.push(String::new());
            lines.extend(entry.time_blocks.iter().map(|block| {
                message!(
                    self.messages,
                    "event.time_block",
                    arg0 = format_time_block(block, self.messages)
                )
            }));
        }
        lines.join("\n")
    }

    fn format_schedule(&self, data: &ItemDisplayData, now: DateTime<Utc>) -> Option<String> {
        let mut entries = data
            .item
            .data
            .iter()
            .filter(|entry| !is_permanent(entry) && header_end(entry) > now)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| header_start(entry));

        struct Group<'a> {
            key: String,
            active: bool,
            period: String,
            entries: Vec<&'a ItemEntry>,
        }
        let mut groups: Vec<Group<'_>> = Vec::new();
        for entry in entries {
            let start = header_start(entry);
            let end = header_end(entry);
            let active = now >= start && now < end;
            let date_key = if active {
                &entry.header.end_date
            } else {
                &entry.header.start_date
            };
            let key = format!("{}:{date_key}", if active { "active" } else { "upcoming" });
            if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                group.entries.push(entry);
            } else {
                groups.push(Group {
                    key,
                    active,
                    period: if active {
                        format!("~{}", format_jst_short(end))
                    } else {
                        format!("{}~", format_jst_short(start))
                    },
                    entries: vec![entry],
                });
            }
        }

        let mut lines = Vec::new();
        for group in groups {
            lines.push(format!(
                "{}[{}]",
                if group.active {
                    message!(self.messages, "event.active")
                } else {
                    ""
                },
                group.period
            ));
            for entry in group.entries {
                let amount = if entry.gift.gift_amount > 0 {
                    message!(
                        self.messages,
                        "event.gift_amount",
                        arg0 = entry.gift.gift_amount
                    )
                } else {
                    String::new()
                };
                lines.push(format!(
                    "    {} {}{amount}",
                    entry.gift.gift_type,
                    schedule_name(entry, data)
                ));
            }
        }
        (!lines.is_empty()).then(|| {
            message!(
                self.messages,
                "event.item_schedule",
                arg0 = lines.join("\n")
            )
        })
    }

    fn format_gift_detail(&self, value: &str) -> String {
        let decoded = decode_html_entities(value);
        let with_breaks = replace_html_breaks(&decoded);
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
        let normalized_newlines = result.replace("\r\n", "\n").replace('\r', "\n");
        let lines = normalized_newlines
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>();
        let mut normalized = lines.join("\n");
        while normalized.contains("\n\n\n") {
            normalized = normalized.replace("\n\n\n", "\n\n");
        }
        normalized.trim().to_owned()
    }

    fn not_found_message(&self, id: i64) -> String {
        message!(self.messages, "event.item_not_found", id = id)
    }
    fn single_message(&self, content: impl Into<String>) -> Result<Vec<String>> {
        let mut output = Vec::new();
        push_message(&mut output, content)?;
        Ok(output)
    }
}
