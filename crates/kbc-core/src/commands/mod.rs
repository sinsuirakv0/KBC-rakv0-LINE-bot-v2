use crate::messages::message;
pub mod content;
pub mod pagination;
pub mod search;
pub mod sessions;

use content::{ContentCatalog, canonical_name};
use search::{SearchCatalog, SearchSession};

pub fn split_responses(
    responses: Vec<sessions::Response>,
) -> crate::Result<Vec<sessions::Response>> {
    let mut result = Vec::new();
    for (body, due, media) in responses {
        if media.is_some() {
            result.push((body, due, media));
            continue;
        }
        let mut remaining = body.as_str();
        while !remaining.is_empty() {
            let mut units = 0;
            let mut end = remaining.len();
            let mut newline = None;
            for (index, character) in remaining.char_indices() {
                units += character.len_utf16();
                if units > 1500 {
                    end = newline.unwrap_or(index);
                    break;
                }
                if character == '\n' {
                    newline = Some(index + 1);
                }
            }
            result.push((remaining[..end].trim_end().to_owned(), due, None));
            remaining = remaining[end..].trim_start_matches('\n');
        }
    }
    if result.len() > 8 {
        return Err("ResponseLimit".into());
    }
    Ok(result)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum CommandPlan {
    Text(Vec<(String, i64)>),
    Search(SearchSession),
    Ignore,
}

pub fn prepare(
    content: &ContentCatalog,
    search_path: &std::path::Path,
    search_live: bool,
    text: &str,
    now: i64,
) -> CommandPlan {
    let message_catalog = &content.messages;
    let text = text.trim();
    let Some(input) = text.strip_prefix('!').or_else(|| text.strip_prefix("o.")) else {
        return CommandPlan::Ignore;
    };
    let mut parts = input.split_whitespace();
    let name = parts.next().unwrap_or_default().to_ascii_lowercase();
    let name = canonical_name(&name);
    let args: Vec<&str> = parts.collect();
    let known = matches!(
        name,
        "pushsetting" | "bot" | "oc" | "id" | "test" | "help" | "ut" | "tut" | "st" | "test-notify"
    ) || content.responses.contains_key(name);
    if !known {
        return CommandPlan::Ignore;
    }
    if name == "help" {
        let target = if name == "help" && !args.is_empty() {
            canonical_name(&args[0].to_ascii_lowercase()).to_owned()
        } else {
            name.to_owned()
        };
        return CommandPlan::Text(vec![(
            content
                .command_help(&target)
                .unwrap_or(message!(message_catalog, "search.prepare_01").into()),
            now,
        )]);
    }
    if name == "id" {
        return CommandPlan::Ignore;
    }
    if matches!(name, "pushsetting" | "test" | "bot" | "oc") {
        return if args.is_empty() {
            CommandPlan::Text(vec![(content.command_help(name).unwrap_or_default(), now)])
        } else {
            CommandPlan::Ignore
        };
    }
    if matches!(name, "ut" | "tut" | "st") {
        if args.is_empty() {
            let page = match name {
                "ut" => "unit",
                "tut" => "tunit",
                _ => "map",
            };
            return CommandPlan::Text(vec![(
                format!("https://jarjarblink.github.io/JDB/{page}_search.html?cc=ja"),
                now,
            )]);
        }
        if args.join(" ").len() > 512 || args.len() > 16 {
            return CommandPlan::Text(vec![(
                message!(message_catalog, "search.prepare_02").into(),
                now,
            )]);
        }
        let search = match SearchCatalog::load(
            search_path,
            std::sync::Arc::clone(message_catalog),
            search_live,
        ) {
            Ok(search) => search,
            Err(error) => {
                eprintln!("Search data unavailable: {error}");
                return CommandPlan::Text(vec![(
                    message!(message_catalog, "search.data_unavailable").into(),
                    now,
                )]);
            }
        };
        return search
            .search(name, &args)
            .map(CommandPlan::Search)
            .unwrap_or_else(|| {
                CommandPlan::Text(vec![(
                    content
                        .command_help(name)
                        .unwrap_or(message!(message_catalog, "search.prepare_03").into()),
                    now,
                )])
            });
    }
    if name == "test-notify" {
        return CommandPlan::Text(match args.first().and_then(|arg| arg.parse::<i64>().ok()) {
            Some(seconds @ 1..=60) if args.len() == 1 => vec![
                (
                    message!(message_catalog, "search.prepare_04", seconds = seconds),
                    now,
                ),
                (
                    message!(message_catalog, "search.prepare_05").into(),
                    now + seconds * 1000,
                ),
            ],
            _ => vec![(message!(message_catalog, "search.prepare_06").into(), now)],
        });
    }
    CommandPlan::Text(vec![(content.responses[name].clone(), now)])
}
