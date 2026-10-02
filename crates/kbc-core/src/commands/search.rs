use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::Result;

pub const PAGE_SIZE: usize = 8;
const MAX_MATCHES: usize = 512;
pub const SESSION_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntry {
    pub id: String,
    pub names: Vec<String>,
    pub lookup_ids: Vec<String>,
    pub url: String,
    #[serde(skip)]
    normalized: Vec<String>,
}

#[derive(Deserialize)]
pub struct SearchCatalog {
    pub revision: String,
    pub entries: HashMap<String, Vec<SearchEntry>>,
}

#[derive(Serialize, Deserialize)]
pub struct SearchSession {
    pub kind: String,
    pub query: String,
    pub results: Vec<usize>,
    pub total: usize,
    pub page: usize,
    pub origin: bool,
    pub form: String,
}

pub fn normalize(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| match character {
            '\u{ff10}'..='\u{ff19}' | '\u{ff21}'..='\u{ff3a}' | '\u{ff41}'..='\u{ff5a}' => {
                char::from_u32(character as u32 - 0xfee0)
                    .unwrap_or(character)
                    .to_ascii_lowercase()
            }
            '\u{30a1}'..='\u{30f6}' => char::from_u32(character as u32 - 0x60).unwrap_or(character),
            '~' | '～' | '〜' => '〜',
            '－' | '−' | '‐' | '⁃' | '‑' | '‒' | '–' | '—' | '―' | '-' => 'ー',
            _ => character,
        })
        .collect()
}

impl SearchCatalog {
    pub fn load(path: &Path) -> Result<Self> {
        if std::fs::metadata(path)?.len() > 4 * 1024 * 1024 {
            return Err("SearchDataLimit".into());
        }
        let text = std::fs::read_to_string(path)?;
        let mut catalog: Self = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        if catalog.revision.len() != 64
            || !catalog
                .revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("InvalidSearchRevision".into());
        }
        // 手編集も含めsnapshotが変わったら、以前の行番号を持つSessionを失効させる。
        let mut fingerprint = DefaultHasher::new();
        text.hash(&mut fingerprint);
        catalog.revision = format!("{:016x}", fingerprint.finish());
        for kind in ["ut", "tut", "st"] {
            let entries = catalog.entries.get_mut(kind).ok_or("MissingSearchData")?;
            if entries.is_empty() || entries.len() > 30000 {
                return Err("SearchEntryLimit".into());
            }
            for entry in entries {
                if entry.id.is_empty()
                    || entry.id.len() > 40
                    || entry.names.is_empty()
                    || entry.names.len() > 16
                    || entry
                        .names
                        .iter()
                        .any(|name| name.is_empty() || name.len() > 512)
                    || entry.lookup_ids.is_empty()
                    || entry.lookup_ids.len() > 4
                    || entry.lookup_ids.iter().any(|id| id.len() > 40)
                    || !entry.url.starts_with("https://jarjarblink.github.io/JDB/")
                    || entry.url.len() > 512
                {
                    return Err("InvalidSearchEntry".into());
                }
                entry.normalized = entry.names.iter().map(|name| normalize(name)).collect();
                for id in &mut entry.lookup_ids {
                    *id = id.to_ascii_lowercase();
                }
            }
        }
        Ok(catalog)
    }

    pub fn search(&self, kind: &str, args: &[&str]) -> SearchSession {
        let origin = kind != "st" && args.iter().any(|arg| arg.eq_ignore_ascii_case("origin"));
        let form = if kind == "ut" && origin {
            args.iter()
                .find(|arg| ["f", "c", "s", "u"].contains(&arg.to_ascii_lowercase().as_str()))
                .map(|arg| arg.to_ascii_lowercase())
                .unwrap_or("f".into())
        } else {
            "f".into()
        };
        let force = args.iter().any(|arg| matches!(*arg, "-f" | "-force"));
        let query = args
            .iter()
            .filter(|arg| {
                !matches!(**arg, "-f" | "-force")
                    && !(origin && arg.eq_ignore_ascii_case("origin"))
                    && !(kind == "ut"
                        && origin
                        && ["f", "c", "s", "u"].contains(&arg.to_ascii_lowercase().as_str()))
            })
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let mut session = SearchSession {
            kind: kind.into(),
            query,
            results: Vec::new(),
            total: 0,
            page: 0,
            origin,
            form,
        };
        if session.query.is_empty() {
            return session;
        }
        let normalized = normalize(&session.query);
        let words: Vec<&str> = if force { &session.query } else { &normalized }
            .split_whitespace()
            .collect();
        // stage IDの区切りは、名前検索の長音正規化と分ける。
        let id_query = session
            .query
            .chars()
            .map(|c| match c {
                '\u{ff10}'..='\u{ff19}' | '\u{ff21}'..='\u{ff3a}' | '\u{ff41}'..='\u{ff5a}' => {
                    char::from_u32(c as u32 - 0xfee0).unwrap_or(c)
                }
                _ => c,
            })
            .collect::<String>()
            .to_ascii_lowercase();
        let id_search = !force
            && !id_query.is_empty()
            && id_query
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            && id_query.bytes().any(|byte| byte.is_ascii_digit());
        let entries = &self.entries[kind];
        if kind != "st"
            && !force
            && let Ok(id) = id_query.parse::<u32>()
            && let Some(index) = entries.iter().position(|entry| entry.id == id.to_string())
        {
            session.results.push(index);
            session.total = 1;
            return session;
        }
        for (index, entry) in entries.iter().enumerate() {
            let id_match = kind == "st"
                && id_search
                && entry.lookup_ids.iter().enumerate().any(|(position, id)| {
                    if id_query.contains('-') {
                        id.contains('-') && id.starts_with(&id_query)
                    } else {
                        !id.contains('-')
                            && if position == 0 {
                                id.starts_with(&id_query)
                            } else {
                                id == &id_query
                            }
                    }
                });
            let name_match = if force && kind != "st" {
                let names = entry.names.join(" ");
                words.iter().all(|word| names.contains(word))
            } else {
                let names = if force {
                    &entry.names
                } else {
                    &entry.normalized
                };
                names
                    .iter()
                    .any(|name| words.iter().all(|word| name.contains(word)))
            };
            if id_match || name_match {
                session.total += 1;
                if session.results.len() < MAX_MATCHES {
                    session.results.push(index);
                }
            }
        }
        session
    }

    pub fn entry<'a>(&'a self, session: &SearchSession, index: usize) -> &'a SearchEntry {
        &self.entries[&session.kind][session.results[index]]
    }

    pub fn page(&self, session: &SearchSession, interactive: bool) -> String {
        let start = session.page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(session.results.len());
        let query: String = session.query.chars().take(60).collect();
        let mut lines = vec![format!(
            "{}「{query}」\n{}〜{} / {}件",
            label(&session.kind),
            start + 1,
            end,
            session.total
        )];
        for index in start..end {
            let entry = self.entry(session, index);
            let name: String = entry.names[0].chars().take(64).collect();
            lines.push(format!("{}．{}  {name}", index - start + 1, entry.id));
        }
        if session.total > session.results.len() {
            lines.push(format!(
                "\n先頭{}件を表示。検索語を追加すると絞れます。",
                session.results.len()
            ));
        }
        if interactive {
            lines.push(format!("\nこの一覧にリプライ\n1〜{}：詳細{}{}\n終了：受付を終える\n検索した本人のみ・10分間\n一覧は操作後・10分経過で削除", end - start, if end < session.results.len() { "　9：次へ" } else { "" }, if start > 0 { "　0：前へ" } else { "" }));
        } else {
            lines.push("\nIDを指定してもう一度検索してください。".into());
        }
        lines.join("\n")
    }
}

pub fn label(kind: &str) -> &'static str {
    match kind {
        "ut" => "ユニット",
        "tut" => "敵ユニット",
        _ => "マップ・ステージ",
    }
}

pub fn detail(entry: &SearchEntry) -> String {
    format!("{}  {}\n{}", entry.id, entry.names[0], entry.url)
}

pub fn image_url(session: &SearchSession, entry: &SearchEntry) -> String {
    let id = entry.id.parse::<u32>().unwrap_or_default();
    if session.kind == "ut" {
        format!(
            "https://jarjarblink.github.io/JDB/static/img/unit_icon/uni{id:03}_{}00.png",
            session.form
        )
    } else {
        format!(
            "https://ponosgames.com/information/appli/battlecats/stage/img/enemy/enemy_icon_{id:03}.png"
        )
    }
}
