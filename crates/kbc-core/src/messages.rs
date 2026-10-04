use std::{collections::BTreeMap, path::Path};

use crate::Result;

pub struct Messages {
    templates: BTreeMap<String, Vec<Part>>,
}

enum Part {
    Text(String),
    Value(String),
}

macro_rules! message {
    ($catalog:expr, $key:literal) => { $catalog.literal($key) };
    ($catalog:expr, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $catalog.text($key, &[$((stringify!($name), ($value).to_string())),+])
    };
}
pub(crate) use message;

impl Messages {
    pub fn status<'a>(&'a self, value: &'a str) -> &'a str {
        match value {
            "成功" => self.literal("common.success"),
            "失敗" => self.literal("common.failed"),
            "結果不明" => self.literal("common.result_unknown"),
            "結果不明（自動再試行しません）" => self.literal("commands.mutation_01"),
            "受付" => self.literal("common.accepted"),
            _ => value,
        }
    }
    pub fn load(root: &Path) -> Result<Self> {
        // 差し込み項目はコード側の契約。文面の変更で新しい変数を要求させない。
        let schema: BTreeMap<String, Vec<String>> =
            serde_json::from_str(include_str!("messages.schema.json"))?;
        let mut templates = BTreeMap::new();
        let mut bytes = 0;
        let mut files = 0;
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|value| value != "txt") {
                continue;
            }
            files += 1;
            bytes += entry.metadata()?.len();
            if !entry.file_type()?.is_file() || files > 16 || bytes > 512 * 1024 {
                return Err("MessageCapacity".into());
            }
            let data = std::fs::read_to_string(entry.path())?;
            for (index, line) in data.trim_start_matches('\u{feff}').lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (key, value) = line.split_once('=').ok_or("InvalidMessageEntry")?;
                let key = key.trim();
                let fields = schema
                    .get(key)
                    .ok_or_else(|| format!("UnknownMessageKey:{key}"))?;
                let value = value.trim();
                let text: String = if value.starts_with('"') {
                    serde_json::from_str(value)?
                } else {
                    value.to_owned()
                };
                if text.len() > 16384 || text.contains("```") {
                    return Err(format!("InvalidMessageText:{key}").into());
                }
                let parts = parse(&text)?;
                if parts
                    .iter()
                    .any(|part| matches!(part, Part::Value(name) if !fields.contains(name)))
                {
                    return Err(format!("UnknownMessageVariable:{key}:{}", index + 1).into());
                }
                if templates.insert(key.to_owned(), parts).is_some() {
                    return Err(format!("DuplicateMessageKey:{key}").into());
                }
            }
        }
        if templates.len() > 1024 || schema.keys().any(|key| !templates.contains_key(key)) {
            return Err("MissingMessageKey".into());
        }
        Ok(Self { templates })
    }

    pub fn text(&self, key: &str, values: &[(&str, String)]) -> String {
        let mut result = String::new();
        for part in &self.templates[key] {
            match part {
                Part::Text(text) => result.push_str(text),
                // 値に含まれる波括弧は再展開せず、名前や本文をそのまま表示する。
                Part::Value(name) => result.push_str(
                    &values
                        .iter()
                        .find(|(key, _)| *key == name)
                        .expect("MissingMessageValue")
                        .1,
                ),
            }
        }
        result
    }

    pub fn literal(&self, key: &str) -> &str {
        match self.templates[key].as_slice() {
            [] => "",
            [Part::Text(text)] => text,
            _ => panic!("MessageRequiresValues:{key}"),
        }
    }
}

fn parse(text: &str) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '{' | '}' if characters.peek() == Some(&character) => {
                literal.push(character);
                characters.next();
            }
            '{' => {
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal)));
                }
                let mut name = String::new();
                loop {
                    match characters.next() {
                        Some('}') => break,
                        Some(value)
                            if value.is_ascii_lowercase()
                                || value.is_ascii_digit()
                                || value == '_' =>
                        {
                            name.push(value)
                        }
                        _ => return Err("InvalidMessageVariable".into()),
                    }
                }
                if name.is_empty() {
                    return Err("InvalidMessageVariable".into());
                }
                parts.push(Part::Value(name));
            }
            '}' => return Err("InvalidMessageBraces".into()),
            value => literal.push(value),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Text(literal));
    }
    Ok(parts)
}
