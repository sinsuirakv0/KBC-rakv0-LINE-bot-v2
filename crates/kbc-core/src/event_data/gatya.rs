//! Discord版の引数解釈と表示をLINEの共通出力へ移植する。
use super::source::{GachaListData as GachaScheduleData, GachaLookupData};
use super::{Source, push_message};
use crate::skd::labels::{mode_for_type, series_id};
use crate::skd::model::*;
use crate::{
    Result,
    messages::{Messages, message},
};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

struct SeriesSummary {
    mode: GachaMode,
    series_id: i64,
    name: String,
    gacha_ids: Vec<i64>,
}

pub(super) async fn run(
    source: &Source,
    messages: &Messages,
    arguments: &[String],
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let formatter = Formatter { messages };
    match parse_request(arguments) {
        GatyaRequest::Schedule { mode } => formatter.single_message(
            formatter
                .format_schedule(&source.fetch_schedule_data().await?, now, mode)
                .unwrap_or_else(|| message!(messages, "event.no_gatya").into()),
        ),
        GatyaRequest::Detail { mode, target } => {
            formatter.format_details(mode, target, &source.fetch_lookup_data().await?)
        }
        GatyaRequest::Search { mode, query } => {
            formatter.format_search(mode, &query, &source.fetch_lookup_data().await?)
        }
        GatyaRequest::Json { mode, target } => {
            formatter
                .format_json_or_raw(mode, target, false, source)
                .await
        }
        GatyaRequest::Raw { mode, target } => {
            formatter
                .format_json_or_raw(mode, target, true, source)
                .await
        }
    }
}

#[derive(Clone, Copy)]
enum GachaTarget {
    Gacha(i64),
    Series(i64),
}

enum GatyaRequest {
    Schedule {
        mode: Option<GachaMode>,
    },
    Detail {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Json {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Raw {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Search {
        mode: Option<GachaMode>,
        query: String,
    },
}

fn parse_request(arguments: &[String]) -> GatyaRequest {
    let (mode, rest) = match arguments.first().and_then(|value| parse_mode(value)) {
        Some(mode) => (Some(mode), &arguments[1..]),
        None => (None, arguments),
    };
    if rest.is_empty() {
        return GatyaRequest::Schedule { mode };
    }
    if rest.len() == 2
        && let Some(target) = parse_target(&rest[0])
    {
        if rest[1].eq_ignore_ascii_case("j") || rest[1].eq_ignore_ascii_case("json") {
            return GatyaRequest::Json { mode, target };
        }
        if rest[1].eq_ignore_ascii_case("r") || rest[1].eq_ignore_ascii_case("raw") {
            return GatyaRequest::Raw { mode, target };
        }
    }
    if rest.len() == 1
        && let Some(target) = parse_target(&rest[0])
    {
        return GatyaRequest::Detail { mode, target };
    }
    GatyaRequest::Search {
        mode,
        query: rest.join(" ").trim().to_owned(),
    }
}

fn parse_mode(value: &str) -> Option<GachaMode> {
    match value.to_ascii_uppercase().as_str() {
        "R" => Some(GachaMode::Rare),
        "E" => Some(GachaMode::Event),
        "N" => Some(GachaMode::Normal),
        _ => None,
    }
}

fn parse_target(value: &str) -> Option<GachaTarget> {
    if let Ok(id) = value.parse() {
        return Some(GachaTarget::Gacha(id));
    }
    let series = value.strip_prefix(['s', 'S'])?;
    if series.is_empty() || !series.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    series.parse().ok().map(GachaTarget::Series)
}

fn find_gacha_blocks(gacha: &GachaJson, id: i64, mode: Option<GachaMode>) -> Vec<&GachaBlock> {
    gacha
        .data
        .iter()
        .filter(|block| {
            mode_matches_type(mode, block.header.gacha_type)
                && block.gachas.iter().any(|entry| entry.id == id)
        })
        .collect()
}

fn find_series_blocks<'a>(
    gacha: &'a GachaJson,
    target_series_id: i64,
    mode: Option<GachaMode>,
    mappings: &MappingMaps,
) -> Vec<&'a GachaBlock> {
    gacha
        .data
        .iter()
        .filter(|block| {
            mode_matches_type(mode, block.header.gacha_type)
                && block.gachas.iter().any(|entry| {
                    series_id(mappings, block.header.gacha_type, entry.id) == Some(target_series_id)
                })
        })
        .collect()
}

fn find_series_summaries(
    target_series_id: i64,
    mode: Option<GachaMode>,
    names: &NameMaps,
    mappings: &MappingMaps,
) -> Vec<SeriesSummary> {
    modes(mode)
        .into_iter()
        .filter_map(|candidate_mode| {
            let gacha_ids = series_member_ids(mappings.get(candidate_mode), target_series_id);
            (!gacha_ids.is_empty()).then(|| SeriesSummary {
                mode: candidate_mode,
                series_id: target_series_id,
                name: names
                    .get(candidate_mode)
                    .get(&target_series_id)
                    .cloned()
                    .unwrap_or_else(|| "不明".to_owned()),
                gacha_ids,
            })
        })
        .collect()
}

fn merge_series_names(full: &NameMaps, short: &NameMaps) -> NameMaps {
    ModeMaps {
        rare: merge_name_map(&full.rare, &short.rare),
        event: merge_name_map(&full.event, &short.event),
        normal: merge_name_map(&full.normal, &short.normal),
    }
}

fn merge_name_map(
    preferred: &HashMap<i64, String>,
    fallback: &HashMap<i64, String>,
) -> HashMap<i64, String> {
    let mut names = fallback.clone();
    names.extend(preferred.iter().map(|(id, name)| (*id, name.clone())));
    names
}

fn series_member_ids(mapping: &HashMap<i64, i64>, target_series_id: i64) -> Vec<i64> {
    let mut ids = mapping
        .iter()
        .filter_map(|(gacha_id, series_id)| (*series_id == target_series_id).then_some(*gacha_id))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn type_for_mode(mode: GachaMode) -> i64 {
    match mode {
        GachaMode::Rare => 1,
        GachaMode::Event => 4,
        GachaMode::Normal => 0,
    }
}

fn mode_label(mode: GachaMode) -> &'static str {
    match mode {
        GachaMode::Rare => "R",
        GachaMode::Event => "E",
        GachaMode::Normal => "N",
    }
}

fn mode_matches_type(mode: Option<GachaMode>, gacha_type: i64) -> bool {
    mode.is_none_or(|mode| type_for_mode(mode) == gacha_type)
}

fn modes(mode: Option<GachaMode>) -> Vec<GachaMode> {
    mode.map_or_else(
        || vec![GachaMode::Rare, GachaMode::Event, GachaMode::Normal],
        |mode| vec![mode],
    )
}

fn gacha_start(block: &GachaBlock) -> DateTime<Utc> {
    parse_header_date(
        &block.header.schedule.start_date,
        &block.header.schedule.start_time,
    )
    .expect("gatya headers are validated when loaded")
}

fn gacha_end(block: &GachaBlock) -> DateTime<Utc> {
    parse_header_date(
        &block.header.schedule.end_date,
        &block.header.schedule.end_time,
    )
    .expect("gatya headers are validated when loaded")
}

fn item_start(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("item headers are validated when loaded")
}

fn item_end(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("item headers are validated when loaded")
}

fn is_permanent(block: &GachaBlock) -> bool {
    block.header.schedule.end_date == "20300101"
}

struct Formatter<'a> {
    messages: &'a Messages,
}
impl Formatter<'_> {
    async fn format_json_or_raw(
        &self,

        mode: Option<GachaMode>,
        target: GachaTarget,
        is_raw: bool,
        data_source: &Source,
    ) -> Result<Vec<String>> {
        let blocks: Vec<GachaBlock> = match target {
            GachaTarget::Series(series_id) => match data_source.fetch_json_with_mappings().await {
                Ok(data) => find_series_blocks(&data.gacha, series_id, mode, &data.series_mappings)
                    .into_iter()
                    .cloned()
                    .collect(),
                Err(error) => return Err(error),
            },
            GachaTarget::Gacha(id) => match data_source.fetch_gacha_json().await {
                Ok(data) => find_gacha_blocks(&data, id, mode)
                    .into_iter()
                    .cloned()
                    .collect(),
                Err(error) => return Err(error),
            },
        };
        if blocks.is_empty() {
            return self.single_message(self.target_error(target));
        }

        let mut output = Vec::new();
        for block in blocks {
            if is_raw {
                match &block.raw {
                    Some(raw) => push_message(&mut output, raw.replace('\t', "    "))?,
                    None => push_message(
                        &mut output,
                        message!(
                            self.messages,
                            "event.raw_missing",
                            arg0 = block.header.schedule.start_date
                        ),
                    )?,
                }
            } else {
                push_message(&mut output, serde_json::to_string_pretty(&block)?)?;
            }
        }
        Ok(output)
    }

    fn format_details(
        &self,

        mode: Option<GachaMode>,
        target: GachaTarget,
        data: &GachaLookupData,
    ) -> Result<Vec<String>> {
        if let GachaTarget::Series(series_id) = target {
            let names = merge_series_names(&data.series_names, &data.short_series_names);
            let summaries = find_series_summaries(series_id, mode, &names, &data.series_mappings);
            if summaries.is_empty() {
                return self.single_message(self.target_error(target));
            }
            return self.single_message(self.format_series_summaries(&summaries));
        }

        let GachaTarget::Gacha(id) = target else {
            unreachable!();
        };
        let blocks = find_gacha_blocks(&data.gacha, id, mode);
        if blocks.is_empty() {
            return self.single_message(self.target_error(target));
        }
        let mut output = Vec::new();
        for block in blocks {
            if let Some(entry) = block.gachas.iter().find(|entry| entry.id == id) {
                push_message(&mut output, self.format_gacha_detail(block, entry, data))?;
            }
        }
        Ok(output)
    }

    fn format_search(
        &self,

        mode: Option<GachaMode>,
        query: &str,
        data: &GachaLookupData,
    ) -> Result<Vec<String>> {
        let names = merge_series_names(&data.series_names, &data.short_series_names);
        let normalized = query.to_lowercase();
        let mut summaries = Vec::new();
        for candidate_mode in modes(mode) {
            let mut matches = names
                .get(candidate_mode)
                .iter()
                .filter(|(_, name)| name.to_lowercase().contains(&normalized))
                .collect::<Vec<_>>();
            matches.sort_by_key(|(series_id, _)| **series_id);
            for (series_id, name) in matches {
                let gacha_ids =
                    series_member_ids(data.series_mappings.get(candidate_mode), *series_id);
                if !gacha_ids.is_empty() {
                    summaries.push(SeriesSummary {
                        mode: candidate_mode,
                        series_id: *series_id,
                        name: name.clone(),
                        gacha_ids,
                    });
                }
            }
        }
        if summaries.is_empty() {
            return self.single_message(message!(
                self.messages,
                "event.series_not_found",
                query = query
            ));
        }
        self.single_message(self.format_series_summaries(&summaries))
    }

    fn format_schedule(
        &self,
        data: &GachaScheduleData,
        now: DateTime<Utc>,
        mode: Option<GachaMode>,
    ) -> Option<String> {
        enum Event<'a> {
            Gacha {
                block: &'a GachaBlock,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            },
            Item {
                entry: &'a ItemEntry,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            },
        }
        impl Event<'_> {
            fn start(&self) -> DateTime<Utc> {
                match self {
                    Self::Gacha { start, .. } | Self::Item { start, .. } => *start,
                }
            }

            fn end(&self) -> DateTime<Utc> {
                match self {
                    Self::Gacha { end, .. } | Self::Item { end, .. } => *end,
                }
            }

            fn date_key(&self, active: bool) -> &str {
                match self {
                    Self::Gacha { block, .. } => {
                        if active {
                            &block.header.schedule.end_date
                        } else {
                            &block.header.schedule.start_date
                        }
                    }
                    Self::Item { entry, .. } => {
                        if active {
                            &entry.header.end_date
                        } else {
                            &entry.header.start_date
                        }
                    }
                }
            }
        }

        let mut events = data
            .gacha
            .data
            .iter()
            .filter(|block| {
                !is_permanent(block)
                    && mode_matches_type(mode, block.header.gacha_type)
                    && gacha_end(block) > now
            })
            .map(|block| Event::Gacha {
                block,
                start: gacha_start(block),
                end: gacha_end(block),
            })
            .collect::<Vec<_>>();
        if !matches!(mode, Some(GachaMode::Event | GachaMode::Normal)) {
            events.extend(
                data.item
                    .data
                    .iter()
                    .filter(|entry| matches!(entry.gift.gift_type, 301 | 302))
                    .filter(|entry| {
                        let start = item_start(entry);
                        if entry.header.end_date == "20300101" {
                            start > now
                        } else {
                            item_end(entry) > now
                        }
                    })
                    .map(|entry| Event::Item {
                        entry,
                        start: item_start(entry),
                        end: item_end(entry),
                    }),
            );
        }
        events.sort_by_key(Event::start);

        struct ScheduleSeries {
            series_id: Option<i64>,
            name: String,
            entries: Vec<GachaEntry>,
            gacha_types: HashSet<i64>,
        }
        struct Group {
            key: String,
            active: bool,
            period: String,
            series: Vec<(String, ScheduleSeries)>,
            gift_items: HashMap<i64, String>,
        }

        let mut groups: Vec<Group> = Vec::new();
        for event in events {
            let start = event.start();
            let end = event.end();
            let active = now >= start && now < end;
            let key = format!(
                "{}:{}",
                if active { "active" } else { "upcoming" },
                event.date_key(active).trim()
            );
            let group_index = groups.iter().position(|group| group.key == key);
            let index = group_index.unwrap_or_else(|| {
                groups.push(Group {
                    key,
                    active,
                    period: if active {
                        format!("~{}", format_jst_short(end))
                    } else {
                        format!("{}~", format_jst_short(start))
                    },
                    series: Vec::new(),
                    gift_items: HashMap::new(),
                });
                groups.len() - 1
            });
            let group = &mut groups[index];

            match event {
                Event::Item { entry, .. } => {
                    group.gift_items.insert(
                        entry.gift.gift_type,
                        data.sale_names
                            .get(&entry.gift.gift_type)
                            .cloned()
                            .unwrap_or_else(|| message!(self.messages, "skd.unknown").to_owned()),
                    );
                }
                Event::Gacha { block, .. } => {
                    let block_mode = mode_for_type(block.header.gacha_type);
                    for entry in &block.gachas {
                        if entry.id < 0 {
                            continue;
                        }
                        let series_id =
                            series_id(&data.series_mappings, block.header.gacha_type, entry.id);
                        let series_key = series_id.map_or_else(
                            || format!("{}:unknown:{}", mode_label(block_mode), entry.id),
                            |id| format!("{}:{id}", mode_label(block_mode)),
                        );
                        let series_index =
                            group.series.iter().position(|(key, _)| *key == series_key);
                        let series_position = series_index.unwrap_or_else(|| {
                            let name = series_id
                                .and_then(|id| data.short_series_names.get(block_mode).get(&id))
                                .cloned()
                                .unwrap_or_else(|| {
                                    message!(self.messages, "skd.unknown").to_owned()
                                });
                            group.series.push((
                                series_key,
                                ScheduleSeries {
                                    series_id,
                                    name,
                                    entries: Vec::new(),
                                    gacha_types: HashSet::new(),
                                },
                            ));
                            group.series.len() - 1
                        });
                        let series = &mut group.series[series_position].1;
                        if let Some(existing) =
                            series.entries.iter_mut().find(|item| item.id == entry.id)
                        {
                            *existing = entry.clone();
                        } else {
                            series.entries.push(entry.clone());
                        }
                        series.gacha_types.insert(block.header.gacha_type);
                    }
                }
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
            let mut gifts = group.gift_items.into_iter().collect::<Vec<_>>();
            gifts.sort_by_key(|(gift_type, _)| *gift_type);
            lines.extend(
                gifts
                    .into_iter()
                    .map(|(gift_type, name)| format!("    {gift_type} {name}")),
            );
            for (_, mut series) in group.series {
                series.entries.sort_by_key(|entry| entry.id);
                let label = series
                    .series_id
                    .map_or_else(|| "s?".to_owned(), |id| format!("s{id}"));
                let tag = if series.gacha_types.len() == 1 {
                    self.type_tag(*series.gacha_types.iter().next().expect("one type"))
                } else {
                    ""
                };
                for entry in series.entries {
                    lines.push(format!(
                        "    {} {label} {}{}{}",
                        entry.id,
                        series.name,
                        self.entry_labels(&entry),
                        tag
                    ));
                }
            }
        }
        (!lines.is_empty()).then(|| {
            message!(
                self.messages,
                "event.gatya_schedule",
                arg0 = lines.join("\n")
            )
        })
    }

    fn format_gacha_detail(
        &self,
        block: &GachaBlock,
        entry: &GachaEntry,
        data: &GachaLookupData,
    ) -> String {
        let mode = mode_for_type(block.header.gacha_type);
        let name = data
            .gacha_names
            .get(mode)
            .get(&entry.id)
            .map(String::as_str)
            .unwrap_or(message!(self.messages, "skd.unknown"));
        let series_label = series_id(&data.series_mappings, block.header.gacha_type, entry.id)
            .map_or_else(|| "s?".to_owned(), |id| format!("s{id}"));
        let end_text = if is_permanent(block) {
            message!(self.messages, "skd.permanent").to_owned()
        } else {
            format_jst_full(gacha_end(block))
        };
        let mut lines = vec![
            message!(
                self.messages,
                "event.gatya_period",
                arg0 = format_jst_full(gacha_start(block)),
                arg1 = end_text,
                arg2 = block.header.schedule.min_version,
                arg3 = block.header.schedule.max_version
            ),
            format!(
                "{} {series_label} {name}{}{}",
                entry.id,
                self.entry_labels(entry),
                self.type_tag(block.header.gacha_type)
            ),
        ];
        let rates = self.format_rates(entry);
        if !rates.is_empty() {
            lines.push(message!(self.messages, "event.rates", rates = rates));
        }
        if let Some(message) = entry
            .message
            .as_deref()
            .filter(|message| !message.is_empty())
        {
            lines.push(message!(
                self.messages,
                "event.gatya_message",
                message = message
            ));
        }
        lines.join("\n")
    }

    fn format_rates(&self, entry: &GachaEntry) -> String {
        [
            (
                message!(self.messages, "event.rate_normal"),
                &entry.rates.normal,
            ),
            (
                message!(self.messages, "event.rate_rare"),
                &entry.rates.rare,
            ),
            (
                message!(self.messages, "event.rate_super_rare"),
                &entry.rates.super_rare,
            ),
            (
                message!(self.messages, "event.rate_uber_rare"),
                &entry.rates.uber_rare,
            ),
            (
                message!(self.messages, "event.rate_legend_rare"),
                &entry.rates.legend_rare,
            ),
        ]
        .into_iter()
        .filter(|(_, value)| value.as_f64() != Some(0.0))
        .map(|(label, value)| format!("{label} {value}"))
        .collect::<Vec<_>>()
        .join(", ")
    }

    fn format_series_summaries(&self, summaries: &[SeriesSummary]) -> String {
        summaries
            .iter()
            .map(|summary| {
                let tag = self.type_tag(type_for_mode(summary.mode));
                summary
                    .gacha_ids
                    .iter()
                    .map(|id| format!("{id} s{} {}{tag}", summary.series_id, summary.name))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn target_error(&self, target: GachaTarget) -> String {
        match target {
            GachaTarget::Gacha(id) => message!(self.messages, "event.gacha_id_not_found", id = id),
            GachaTarget::Series(id) => {
                message!(self.messages, "event.series_id_not_found", id = id)
            }
        }
    }

    fn entry_labels(&self, entry: &GachaEntry) -> String {
        crate::skd::labels::entry_labels(entry, self.messages)
    }

    fn type_tag(&self, gacha_type: i64) -> &str {
        crate::skd::labels::type_tag(gacha_type, self.messages)
    }
    fn single_message(&self, content: impl Into<String>) -> Result<Vec<String>> {
        let mut output = Vec::new();
        push_message(&mut output, content)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn includes_super_rare_in_detail_rates() {
        let messages = Messages::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/messages"),
        )
        .unwrap();
        let entry = serde_json::from_value(serde_json::json!({
            "id": 1, "price": 150, "flags": 0, "guaranteed": false,
            "rates": {"normal": 0, "rare": 6970, "superRare": 2500, "uberRare": 500, "legendRare": 30}
        })).unwrap();
        assert_eq!(
            Formatter {
                messages: &messages
            }
            .format_rates(&entry),
            "レア 6970, 激レア 2500, 超激レア 500, 伝説レア 30"
        );
    }
}
