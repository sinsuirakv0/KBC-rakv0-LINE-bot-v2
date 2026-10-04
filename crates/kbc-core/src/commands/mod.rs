pub mod content;
pub mod search;
pub mod sessions;

use content::{ContentCatalog, canonical_name};
use search::{SearchCatalog, SearchSession};

pub fn message_emojis(text: &str) -> Option<Vec<kbc_protocol::MessageEmoji>> {
    // 標準LINE絵文字の案内部分だけ装飾する。位置は分割後の本文のUTF-16単位。
    let mut result = Vec::new();
    for (label, symbol, emoji_id) in [("👍（いいね）", "👍", "143"), ("❤️（ハート）", "❤️", "165")]
    {
        for (index, _) in text.match_indices(label) {
            let start = text[..index].encode_utf16().count() as u32;
            result.push(kbc_protocol::MessageEmoji {
                product_id: "670e0cce840a8236ddd4ee4c".into(),
                emoji_id: emoji_id.into(),
                start,
                end: start + symbol.encode_utf16().count() as u32,
            });
        }
    }
    result.sort_by_key(|item| item.start);
    result.truncate(20);
    (!result.is_empty()).then_some(result)
}

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
    search: &SearchCatalog,
    text: &str,
    now: i64,
) -> CommandPlan {
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
        "bot" | "oc" | "id" | "test" | "help" | "ut" | "tut" | "st" | "test-notify"
    ) || content.responses.contains_key(name);
    if !known {
        return CommandPlan::Ignore;
    }
    if name == "help"
        || args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("help"))
    {
        let target = if name == "help" && !args.is_empty() {
            canonical_name(args[0])
        } else {
            name
        };
        return CommandPlan::Text(vec![(
            content
                .command_help(target)
                .unwrap_or("そのコマンドの案内はありません。!help で一覧を確認できます。".into()),
            now,
        )]);
    }
    if name == "id" {
        return CommandPlan::Ignore;
    }
    if name == "test" {
        return if args.first().is_some_and(|arg| {
            [
                "reply", "allow", "mention", "delete", "kick", "deputy", "admin",
            ]
            .iter()
            .any(|name| arg.eq_ignore_ascii_case(name))
        }) && !args
            .get(1)
            .is_some_and(|arg| arg.eq_ignore_ascii_case("help"))
        {
            CommandPlan::Ignore
        } else {
            CommandPlan::Text(vec![(
                content.command_help("test").unwrap_or_default(),
                now,
            )])
        };
    }
    if matches!(name, "bot" | "oc") {
        return CommandPlan::Text(vec![(content.command_help(name).unwrap_or_default(), now)]);
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
                "検索語が長すぎます。短くしてもう一度お試しください。".into(),
                now,
            )]);
        }
        return search
            .search(name, &args)
            .map(CommandPlan::Search)
            .unwrap_or_else(|| {
                CommandPlan::Text(vec![(
                    content
                        .command_help(name)
                        .unwrap_or("引数を確認してください。".into()),
                    now,
                )])
            });
    }
    if name == "test-notify" {
        return CommandPlan::Text(match args.first().and_then(|arg| arg.parse::<i64>().ok()) {
            Some(seconds @ 1..=60) if args.len() == 1 => vec![
                (format!("{seconds}秒後に通知を送ります。"), now),
                (
                    "通知の確認です。次の入力がなくても送信されます。".into(),
                    now + seconds * 1000,
                ),
            ],
            _ => vec![("確認用: !test-notify 1〜60（秒）".into(), now)],
        });
    }
    CommandPlan::Text(vec![(content.responses[name].clone(), now)])
}
