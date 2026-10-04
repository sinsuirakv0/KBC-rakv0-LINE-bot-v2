use crate::{
    Result,
    motion::{
        MotionPlan,
        request::{MotionRequest, parse_motion_arguments},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
};
use unicode_normalization::UnicodeNormalization;
pub const PAGE_SIZE: usize = 8;
pub const SESSION_TTL_MS: i64 = 600000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntry {
    pub id: String,
    pub display_name: String,
    pub names: Vec<String>,
    pub lookup_ids: Vec<String>,
    pub url: String,
    #[serde(default)]
    pub raw_url: Option<String>,
    #[serde(default)]
    pub form_count: usize,
    #[serde(default)]
    pub shared_forms: [Option<String>; 2],
    #[serde(skip)]
    normalized: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCatalog {
    source: SnapshotSource,
    schema_version: u32,
    pub revision: String,
    pub entries: HashMap<String, Vec<SearchEntry>>,
    #[serde(skip)]
    ids: HashMap<String, HashMap<String, usize>>,
    #[serde(skip)]
    display_types: Vec<String>,
}

#[derive(Deserialize)]
struct SnapshotSource {
    repositories: HashMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Operation {
    Detail,
    Origin { family: String, form: String },
    File { form: Option<String> },
    Motion(MotionRequest),
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub index: usize,
    pub name: Option<usize>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FileOption {
    pub path: String,
    pub label: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SearchSession {
    pub kind: String,
    pub query: String,
    pub results: Vec<SearchMatch>,
    pub total: usize,
    pub page: usize,
    pub operation: Operation,
    #[serde(default)]
    pub files: Option<Vec<FileOption>>,
}
// Discordと同じNFKC・大小文字・かな・長音・波線の正規化。
pub fn normalize(value: &str) -> String {
    value
        .trim()
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '\u{30a1}'..='\u{30f6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            '~' | '～' | '〜' => '〜',
            '－' | '−' | '‐' | '⁃' | '‑' | '‒' | '–' | '—' | '―' | '-' => 'ー',
            _ => c,
        })
        .collect()
}
fn origin(args: &[&str]) -> Option<Operation> {
    let values: Vec<String> = args.iter().map(|arg| arg.to_ascii_lowercase()).collect();
    let (family, form) = match values.as_slice() {
        [] => ("icon", "f"),
        [form] if ["f", "c", "s", "u"].contains(&form.as_str()) => ("icon", form.as_str()),
        [family] if ["icon", "wide", "sprite", "gacha"].contains(&family.as_str()) => {
            (family.as_str(), "f")
        }
        [family, form]
            if ["icon", "wide", "sprite"].contains(&family.as_str())
                && ["f", "c", "s", "u"].contains(&form.as_str()) =>
        {
            (family.as_str(), form.as_str())
        }
        [family, form] if family == "gacha" && ["m", "z"].contains(&form.as_str()) => {
            ("gacha", form.as_str())
        }
        _ => return None,
    };
    Some(Operation::Origin {
        family: family.into(),
        form: form.into(),
    })
}
fn parse(kind: &str, args: &[&str]) -> Option<(String, bool, Operation)> {
    if kind == "st" {
        let force = args
            .first()
            .is_some_and(|a| a.eq_ignore_ascii_case("-f") || a.eq_ignore_ascii_case("-force"));
        return Some((
            args[usize::from(force)..].join(" "),
            force,
            Operation::Detail,
        ));
    }
    let operations: Vec<usize> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| ["origin", "file", "motion"].contains(&a.to_ascii_lowercase().as_str()))
        .map(|(i, _)| i)
        .collect();
    if operations.len() > 1 {
        return None;
    }
    let index = operations.first().copied().unwrap_or(args.len());
    let mut force = false;
    let mut query = Vec::new();
    for arg in &args[..index] {
        if arg.eq_ignore_ascii_case("-f") || (kind == "tut" && arg.eq_ignore_ascii_case("-force")) {
            force = true;
        } else {
            query.push(*arg);
        }
    }
    let mut tail = args.get(index + 1..).unwrap_or_default().to_vec();
    if kind == "tut" {
        tail.retain(|a| {
            let flag = a.eq_ignore_ascii_case("-f") || a.eq_ignore_ascii_case("-force");
            force |= flag;
            !flag
        });
    }
    let operation = if index == args.len() {
        Operation::Detail
    } else {
        match args[index].to_ascii_lowercase().as_str() {
            "origin" if kind == "tut" => {
                query.extend(tail);
                Operation::Origin {
                    family: "icon".into(),
                    form: "e".into(),
                }
            }
            "origin" => origin(&tail)?,
            "file" => Operation::File {
                form: match tail.as_slice() {
                    [] => None,
                    [f] if kind == "ut"
                        && ["f", "c", "s", "u"].contains(&f.to_ascii_lowercase().as_str()) =>
                    {
                        Some(f.to_ascii_lowercase())
                    }
                    _ => return None,
                },
            },
            "motion" => Operation::Motion(parse_motion_arguments(
                &tail.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                if kind == "ut" {
                    &["f", "c", "s", "u"]
                } else {
                    &[]
                },
            )?),
            _ => return None,
        }
    };
    let query = query.join(" ");
    (!query.is_empty()).then_some((query, force, operation))
}
impl SearchCatalog {
    pub fn asset_url(&self, path: &str) -> Result<String> {
        Ok(format!(
            "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{}/jp/sitedata/{path}",
            self.asset_commit()?
        ))
    }
    pub fn asset_commit(&self) -> Result<&str> {
        self.source
            .repositories
            .get("sinsuirakv0/KBC-rakv0-assets")
            .map(String::as_str)
            .ok_or("MissingAssetCommit".into())
    }
    pub fn load(path: &Path) -> Result<Self> {
        if std::fs::metadata(path)?.len() > 4 * 1024 * 1024 {
            return Err("SearchDataLimit".into());
        }
        let text = std::fs::read_to_string(path)?;
        let mut catalog: Self = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        if catalog.schema_version != 2
            || catalog.revision.len() != 64
            || !catalog.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("InvalidSearchRevision".into());
        }
        let mut fingerprint = DefaultHasher::new();
        text.hash(&mut fingerprint);
        catalog.revision = format!("{:016x}", fingerprint.finish());
        for kind in ["ut", "tut", "st"] {
            let entries = catalog.entries.get_mut(kind).ok_or("MissingSearchData")?;
            if entries.is_empty() || entries.len() > 30000 {
                return Err("SearchEntryLimit".into());
            }
            let mut ids = HashMap::new();
            for (index, entry) in entries.iter_mut().enumerate() {
                if entry.id.is_empty()
                    || entry.id.len() > 40
                    || entry.names.is_empty()
                    || entry.names.len() > 32
                    || entry.display_name.is_empty()
                    || entry.display_name.len() > 512
                    || entry.names.iter().any(|n| n.is_empty() || n.len() > 512)
                    || entry.lookup_ids.is_empty()
                    || entry.lookup_ids.len() > 4
                    || !entry.url.starts_with("https://jarjarblink.github.io/JDB/")
                    || entry.url.len() > 512
                    || entry.raw_url.as_ref().is_some_and(|u| {
                        !u.starts_with("https://jarjarblink.github.io/JDB/") || u.len() > 512
                    })
                    || (kind == "ut"
                        && (!(1..=4).contains(&entry.form_count)
                            || entry.form_count > entry.names.len()
                            || entry
                                .shared_forms
                                .iter()
                                .flatten()
                                .any(|id| id.len() > 8 || !id.bytes().all(|b| b.is_ascii_digit()))))
                {
                    return Err("InvalidSearchEntry".into());
                }
                entry.normalized = entry.names.iter().map(|n| normalize(n)).collect();
                for id in &entry.lookup_ids {
                    if id.len() > 80 || ids.insert(id.to_ascii_lowercase(), index).is_some() {
                        return Err("DuplicateSearchId".into());
                    }
                    if kind == "st"
                        && let Some(value) = id.strip_prefix("type:")
                        && let Some((name, _)) = value.split_once(':')
                    {
                        catalog.display_types.push(name.into());
                    }
                }
            }
            catalog.ids.insert(kind.into(), ids);
        }
        catalog
            .display_types
            .sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        catalog.display_types.dedup();
        Ok(catalog)
    }
    fn stage_key(&self, query: &str) -> Option<String> {
        if query.is_empty()
            || !query
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return None;
        }
        let (base, stage) = match query.split_once('-') {
            Some((base, stage)) if !stage.contains('-') => {
                (base, Some(stage.parse::<usize>().ok()?))
            }
            Some(_) => return None,
            None => (query, None),
        };
        let key = if base.bytes().all(|c| c.is_ascii_digit()) {
            format!("raw:{}", base.parse::<usize>().ok()?)
        } else {
            let lower = base.to_ascii_lowercase();
            let name = self
                .display_types
                .iter()
                .find(|n| lower.starts_with(n.as_str()))?;
            format!(
                "type:{name}:{}",
                base.get(name.len()..)?.parse::<usize>().ok()?
            )
        };
        Some(stage.map_or(key.clone(), |s| format!("{key}:stage:{s}")))
    }
    pub fn search(&self, kind: &str, args: &[&str]) -> Option<SearchSession> {
        let (query, force, operation) = parse(kind, args)?;
        let mut session = SearchSession {
            kind: kind.into(),
            query,
            results: Vec::new(),
            total: 0,
            page: 0,
            operation,
            files: None,
        };
        // ASCIIの完全ID解決。force時のID解決はDiscordと同じくtutだけ維持する。
        let id = if kind == "st" && !force {
            self.stage_key(&session.query)
        } else if kind != "st"
            && (!force || kind == "tut")
            && session.query.bytes().all(|c| c.is_ascii_digit())
        {
            session.query.parse::<usize>().ok().map(|i| i.to_string())
        } else {
            None
        };
        if let Some(index) = id.and_then(|id| self.ids[kind].get(&id)) {
            session.results.push(SearchMatch {
                index: *index,
                name: None,
            });
            session.total = 1;
            return Some(session);
        }
        let normalized = if force {
            session.query.clone()
        } else {
            normalize(&session.query)
        };
        let words: Vec<&str> = normalized.split_whitespace().collect();
        for (index, entry) in self.entries[kind].iter().enumerate() {
            let names = if force {
                &entry.names
            } else {
                &entry.normalized
            };
            let eligible = if kind == "ut" && force {
                &names[..entry.form_count]
            } else {
                names.as_slice()
            };
            if let Some(name) = eligible
                .iter()
                .position(|n| words.iter().all(|w| n.contains(w)))
            {
                session.total += 1;
                if session.results.len() < 512 {
                    session.results.push(SearchMatch {
                        index,
                        name: Some(name),
                    });
                }
            }
        }
        Some(session)
    }
    pub fn entry<'a>(&'a self, s: &SearchSession, index: usize) -> &'a SearchEntry {
        &self.entries[&s.kind][s.results[index].index]
    }
    pub fn entry_label(&self, s: &SearchSession, index: usize) -> String {
        let e = self.entry(s, index);
        let matched = s.results[index].name;
        let name = if s.kind == "tut" && e.display_name == "ダミー" {
            matched
                .and_then(|i| e.names.get(i))
                .filter(|n| n.as_str() != "ダミー")
                .or_else(|| e.names.iter().find(|n| n.as_str() != "ダミー"))
                .map(|n| format!("{n} (ダミー)"))
                .unwrap_or("ダミー".into())
        } else {
            e.display_name.clone()
        };
        let annotation = if s.kind == "ut" {
            match matched {
                Some(n) if n >= e.form_count => " (別称でヒット)",
                Some(1) => " (第二形態名でヒット)",
                Some(2) => " (第三形態名でヒット)",
                Some(3) => " (第四形態名でヒット)",
                _ => "",
            }
        } else {
            ""
        };
        format!(
            "{} {}{annotation}",
            e.id,
            name.chars().take(96).collect::<String>()
        )
    }
    pub fn detail(&self, s: &SearchSession, index: usize) -> String {
        let e = self.entry(s, index);
        let url = if s.kind == "st" && s.results[index].name.is_none() {
            e.raw_url.as_ref().unwrap_or(&e.url)
        } else {
            &e.url
        };
        format!("{}\n{url}", self.entry_label(s, index))
    }
    pub fn page(&self, s: &SearchSession, interactive: bool) -> String {
        let count = s.files.as_ref().map_or(s.results.len(), Vec::len);
        let start = s.page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(count);
        let mut lines = vec![format!(
            "{}「{}」\n{}〜{} / {}件",
            label(&s.kind),
            s.query.chars().take(60).collect::<String>(),
            start + 1,
            end,
            if s.files.is_some() { count } else { s.total }
        )];
        for i in start..end {
            let name = if let Some(files) = &s.files {
                files[i].label.clone()
            } else {
                self.entry_label(s, i)
            };
            let mut short = String::new();
            let mut width = 0;
            for ch in name.chars() {
                if width + ch.len_utf16() > 96 {
                    short.push('…');
                    break;
                }
                short.push(ch);
                width += ch.len_utf16();
            }
            lines.push(format!("{}．{short}", i - start + 1));
        }
        if s.files.is_none() && s.total > count {
            lines.push(format!(
                "\n先頭{count}件を表示。検索語を追加すると絞れます。"
            ));
        }
        if interactive {
            lines.push(format!(
                "\nこの一覧へ番号をリプライ\n1〜{}：{}",
                end - start,
                if s.files.is_some() {
                    "ファイル"
                } else {
                    "詳細"
                }
            ));
            let mut moves = Vec::new();
            if end < count {
                moves.push("👍（いいね）：次へ");
            }
            if start > 0 {
                moves.push("❤️（ハート）：前へ");
            }
            if !moves.is_empty() {
                lines.push(format!("リアクション：{}", moves.join("　")));
            }
            lines.push(
                "終了：受付を終える\n検索した本人のみ・10分間\n一覧は操作後・10分経過で削除".into(),
            );
        } else {
            lines.push("\nIDを指定してもう一度検索してください。".into());
        }
        lines.join("\n")
    }
}
fn stem(e: &SearchEntry, form: &str) -> Option<(String, String, bool)> {
    let index = ["f", "c", "s", "u"].iter().position(|f| *f == form)?;
    if index >= e.form_count {
        return None;
    }
    if let Some(id) = e.shared_forms.get(index).and_then(Option::as_ref) {
        Some((id.clone(), "m".into(), true))
    } else {
        Some((e.id.clone(), form.into(), false))
    }
}
pub fn origin_path(e: &SearchEntry, kind: &str, family: &str, form: &str) -> Option<String> {
    if kind == "tut" {
        return Some(format!(
            "Image/enemy_icon_{:03}.png",
            e.id.parse::<usize>().ok()?
        ));
    }
    if family == "gacha" {
        return Some(format!("Image/gatyachara_{}_{form}.png", e.id));
    }
    let (id, suffix, shared) = stem(e, form)?;
    if family == "sprite" {
        return Some(format!("Number/{id}_{suffix}.png"));
    }
    let suffix = if shared {
        format!("_m0{}.png", usize::from(form == "c"))
    } else if family == "icon" {
        format!("_{form}00.png")
    } else {
        format!("_{form}.png")
    };
    Some(format!(
        "Unit/{}{id}{suffix}",
        if family == "icon" { "uni" } else { "udi" }
    ))
}
pub fn file_options(e: &SearchEntry, kind: &str, filter: Option<&str>) -> Vec<FileOption> {
    let mut result = Vec::new();
    let mut add = |path, label| result.push(FileOption { path, label });
    let forms: Vec<&str> = if kind == "tut" {
        vec!["e"]
    } else {
        ["f", "c", "s", "u"]
            .iter()
            .copied()
            .take(e.form_count)
            .filter(|f| filter.is_none_or(|v| v == *f))
            .collect()
    };
    for form in forms {
        let (id, suffix, label) = if kind == "tut" {
            (
                format!("{:03}", e.id.parse::<usize>().unwrap_or_default()),
                "e".into(),
                "敵".into(),
            )
        } else {
            let Some((id, suffix, _)) = stem(e, form) else {
                continue;
            };
            (
                id,
                suffix,
                format!(
                    "第{}形態",
                    ["f", "c", "s", "u"]
                        .iter()
                        .position(|f| *f == form)
                        .unwrap()
                        + 1
                ),
            )
        };
        if kind == "tut" {
            add(format!("Image/enemy_icon_{id}.png"), "敵アイコン".into());
        } else {
            for (family, title) in [("icon", "アイコン"), ("wide", "横長画像")] {
                if let Some(path) = origin_path(e, kind, family, form) {
                    add(path, format!("{label} {title}"));
                }
            }
        }
        let base = format!("{id}_{suffix}");
        add(format!("Number/{base}.png"), format!("{label} スプライト"));
        add(
            format!("ImageData/{base}.imgcut"),
            format!("{label} 切り抜き情報"),
        );
        add(
            format!("ImageData/{base}.mamodel"),
            format!("{label} モデル"),
        );
        for (i, title) in ["歩行", "待機", "攻撃", "ノックバック"].iter().enumerate() {
            add(
                format!("ImageData/{base}0{i}.maanim"),
                format!("{label} {title}"),
            );
        }
    }
    if kind == "ut" && filter.is_none() {
        for form in ["f", "m", "z"] {
            add(
                format!("Image/gatyachara_{}_{form}.png", e.id),
                format!("ガチャ画像 {form}"),
            );
        }
    }
    result
}
pub fn motion_plan(e: &SearchEntry, kind: &str, r: &MotionRequest) -> Option<MotionPlan> {
    let form = r
        .form
        .as_deref()
        .unwrap_or(if kind == "ut" { "f" } else { "e" });
    let (id, suffix) = if kind == "ut" {
        let (id, suffix, _) = stem(e, form)?;
        (id, suffix)
    } else {
        (format!("{:03}", e.id.parse::<usize>().ok()?), "e".into())
    };
    let base = format!("{id}_{suffix}");
    Some(MotionPlan {
        format: r.format,
        full: r.full,
        filename_stem: format!("{kind}-{}-{form}-motion", e.id),
        preview_scale: match (kind, e.id.as_str(), form) {
            ("ut", "000", "f") | ("tut", "0", "e") => 2.25,
            ("ut", "009", "f") => 0.82,
            _ => 1.0,
        },
        segments: r.segments.clone(),
        sprite_path: format!("Number/{base}.png"),
        imgcut_path: format!("ImageData/{base}.imgcut"),
        model_path: format!("ImageData/{base}.mamodel"),
        animation_paths: r
            .segments
            .iter()
            .map(|s| {
                (
                    s.motion,
                    format!("ImageData/{base}0{}.maanim", s.motion.asset_index()),
                )
            })
            .collect(),
    })
}
pub fn label(kind: &str) -> &'static str {
    match kind {
        "ut" => "ユニット",
        "tut" => "敵ユニット",
        _ => "マップ・ステージ",
    }
}
