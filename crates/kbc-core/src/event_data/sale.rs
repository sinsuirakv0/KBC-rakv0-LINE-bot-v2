//! Discord版の引数解釈と表示をLINEの共通出力へ移植する。
use super::{Output, Selection, format_time_block};
use super::{Source, push_message};
use crate::skd::labels::{is_mission_id, strip_display_markup};
use crate::skd::labels::{list_stage_ids, representative_stage_id, stage_name};
use crate::skd::model::*;
use crate::{
    Result,
    messages::{Messages, message},
};
use chrono::{DateTime, Utc};

pub(super) async fn run(
    source: &Source,
    messages: &Messages,
    arguments: &[String],
    now: DateTime<Utc>,
    can_select: bool,
) -> Result<Output> {
    let formatter = Formatter { messages };
    let texts = match parse_request(arguments) {
        SaleRequest::Schedule => formatter.single_message(
            formatter
                .format_schedule(&source.sale().await?, now)
                .unwrap_or_else(|| message!(messages, "event.no_sale").into()),
        )?,
        SaleRequest::Detail { id } => formatter.format_details(id, &source.sale().await?)?,
        SaleRequest::Json { id } => {
            formatter.format_json_or_raw(id, false, &source.fetch_sale_json().await?)?
        }
        SaleRequest::Raw { id } => {
            formatter.format_json_or_raw(id, true, &source.fetch_sale_json().await?)?
        }
        SaleRequest::Search { query } => {
            return formatter.format_search(&query, &source.sale().await?, can_select);
        }
    };
    Ok(Output::text(texts))
}

enum SaleRequest {
    Schedule,
    Detail { id: i64 },
    Json { id: i64 },
    Raw { id: i64 },
    Search { query: String },
}

fn parse_request(arguments: &[String]) -> SaleRequest {
    if arguments.is_empty() {
        return SaleRequest::Schedule;
    }
    if let Ok(id) = arguments[0].trim().parse() {
        return match arguments.get(1).map(|value| value.to_ascii_lowercase()) {
            Some(modifier) if modifier == "j" || modifier == "json" => SaleRequest::Json { id },
            Some(modifier) if modifier == "r" || modifier == "raw" => SaleRequest::Raw { id },
            _ => SaleRequest::Detail { id },
        };
    }
    SaleRequest::Search {
        query: arguments.join(" ").trim().to_owned(),
    }
}

fn normalized_date_key(value: &str) -> String {
    format!("{:0>8}", value.trim())
}

fn header_start(entry: &SaleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("sale headers are validated when loaded")
}

fn header_end(entry: &SaleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("sale headers are validated when loaded")
}

fn is_permanent(entry: &SaleEntry) -> bool {
    entry.header.end_date == "20300101"
}

struct Formatter<'a> {
    messages: &'a Messages,
}
impl Formatter<'_> {
    fn format_search(
        &self,
        query: &str,
        data: &SaleDisplayData,
        can_select: bool,
    ) -> Result<Output> {
        let mut names = data.all_day_event_names.clone();
        names.extend(data.sale_names.clone());
        let normalized = query.to_lowercase();
        let mut matches = names
            .into_iter()
            .filter(|(id, name)| !is_mission_id(*id) && name.to_lowercase().contains(&normalized))
            .collect::<Vec<_>>();
        matches.sort_by_key(|(id, _)| *id);
        if matches.is_empty() {
            return Ok(Output::text(self.single_message(message!(
                self.messages,
                "event.sale_not_found",
                query = query
            ))?));
        }
        let selectable = can_select && matches.len() <= 9;
        let lines = matches
            .iter()
            .enumerate()
            .map(|(index, (id, name))| {
                let name = strip_display_markup(name, " ");
                if selectable {
                    format!("{}. {id} {name}", index + 1)
                } else {
                    format!("{id} {name}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut text = message!(
            self.messages,
            "event.sale_results",
            query = query,
            count = matches.len(),
            lines = lines
        );
        let selectable = selectable
            && text.encode_utf16().count()
                + message!(self.messages, "event.select_hint")
                    .encode_utf16()
                    .count()
                <= 1500;
        if selectable {
            text.push_str(message!(self.messages, "event.select_hint"));
        }
        Ok(Output {
            messages: self.single_message(text)?,
            selection: selectable.then(|| Selection {
                kind: "sale".into(),
                choices: matches.into_iter().map(|(id, _)| id).collect(),
            }),
        })
    }
    fn format_details(&self, id: i64, data: &SaleDisplayData) -> Result<Vec<String>> {
        let entries = data
            .sale
            .data
            .iter()
            .filter(|entry| entry.stage_ids.contains(&id))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return self.single_message(message!(
                self.messages,
                "event.sale_id_not_found",
                id = id
            ));
        }
        let mut output = Vec::new();
        for entry in entries {
            push_message(&mut output, self.format_detail(entry, data, id))?;
        }
        Ok(output)
    }

    fn format_json_or_raw(&self, id: i64, is_raw: bool, sale: &SaleJson) -> Result<Vec<String>> {
        let entries = sale
            .data
            .iter()
            .filter(|entry| entry.stage_ids.contains(&id))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return self.single_message(message!(
                self.messages,
                "event.sale_id_not_found",
                id = id
            ));
        }
        let mut output = Vec::new();
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

    fn format_detail(&self, entry: &SaleEntry, data: &SaleDisplayData, selected_id: i64) -> String {
        let mut lines = Vec::new();
        let representative_id = representative_stage_id(entry, &data.card_setting_stage_ids);
        if representative_id == Some(selected_id) {
            lines.push(format!("{selected_id} {}", stage_name(selected_id, data)));
            let targets = entry
                .stage_ids
                .iter()
                .filter(|id| **id != selected_id)
                .map(|id| format!("{id} {}", stage_name(*id, data)))
                .collect::<Vec<_>>();
            if !targets.is_empty() {
                lines.push(message!(
                    self.messages,
                    "event.target_stages",
                    arg0 = targets.join(message!(self.messages, "event.separator"))
                ));
            }
        } else {
            lines.push(format!("{selected_id} {}", stage_name(selected_id, data)));
        }

        let start = header_start(entry);
        let end_text = if is_permanent(entry) {
            message!(self.messages, "skd.permanent").to_owned()
        } else {
            format_jst_full(header_end(entry))
        };
        lines.push(format!(
            "{} ~ {}  ver.{}~{}",
            format_jst_full(start),
            end_text,
            entry.header.min_version,
            entry.header.max_version
        ));
        if entry.time_blocks.is_empty() {
            lines.push(message!(self.messages, "event.unlimited_time").to_owned());
        } else {
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

    fn format_schedule(&self, data: &SaleDisplayData, now: DateTime<Utc>) -> Option<String> {
        let mut entries = data
            .sale
            .data
            .iter()
            .filter(|entry| !is_permanent(entry) && header_end(entry) > now)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| header_start(entry));

        struct ScheduleItem {
            active: bool,
            period: String,
            start_date_key: String,
            end_date_key: String,
            stage_ids: Vec<i64>,
            duration: String,
        }
        struct Group {
            key: String,
            active: bool,
            period: String,
            items: Vec<ScheduleItem>,
        }

        let mut groups: Vec<Group> = Vec::new();
        for entry in entries {
            let stage_ids = list_stage_ids(entry, &data.card_setting_stage_ids);
            if stage_ids.is_empty() {
                continue;
            }
            let start = header_start(entry);
            let end = header_end(entry);
            let active = now >= start && now < end;
            let item = ScheduleItem {
                active,
                period: if active {
                    format!("~{}", format_jst_short(end))
                } else {
                    format!("{}~", format_jst_short(start))
                },
                start_date_key: normalized_date_key(&entry.header.start_date),
                end_date_key: normalized_date_key(&entry.header.end_date),
                stage_ids,
                duration: format_duration(start, end),
            };
            let date_key = if active {
                &item.end_date_key
            } else {
                &item.start_date_key
            };
            let key = format!("{}:{date_key}", if active { "active" } else { "upcoming" });
            if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                group.items.push(item);
            } else {
                groups.push(Group {
                    key,
                    active: item.active,
                    period: item.period.clone(),
                    items: vec![item],
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
            for item in group.items {
                for id in item.stage_ids {
                    lines.push(format!(
                        "    {id} {} {}",
                        stage_name(id, data),
                        item.duration
                    ));
                }
            }
        }
        (!lines.is_empty()).then(|| {
            message!(
                self.messages,
                "event.sale_schedule",
                arg0 = lines.join("\n")
            )
        })
    }
    fn single_message(&self, content: impl Into<String>) -> Result<Vec<String>> {
        let mut output = Vec::new();
        push_message(&mut output, content)?;
        Ok(output)
    }
}
