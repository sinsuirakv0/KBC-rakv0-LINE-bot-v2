use std::{collections::BTreeMap, path::Path};

use crate::Result;

pub struct ContentCatalog {
    pub responses: BTreeMap<String, String>,
    help: BTreeMap<String, String>,
}

pub fn canonical_name(name: &str) -> &str {
    match name {
        "unit" => "ut",
        "enemy" => "tut",
        "stage" => "st",
        _ => name,
    }
}

impl ContentCatalog {
    pub fn load(root: &Path) -> Result<Self> {
        let responses = read_folder(&root.join("responses"))?;
        if responses.keys().any(|key| {
            matches!(
                key.as_str(),
                "oc" | "oc-admin"
                    | "help"
                    | "ut"
                    | "tut"
                    | "st"
                    | "unit"
                    | "enemy"
                    | "stage"
                    | "test-notify"
            )
        }) {
            return Err("ReservedContentName".into());
        }
        let help = read_folder(&root.join("help"))?;
        for key in ["index", "oc", "oc-admin", "ut", "tut", "st", "test-notify"] {
            if !help.contains_key(key) {
                return Err(format!("MissingHelp:{key}").into());
            }
        }
        Ok(Self { responses, help })
    }

    pub fn internal_help(&self, name: &str) -> Option<String> {
        self.help.get(name).cloned()
    }

    pub fn command_help(&self, name: &str) -> Option<String> {
        if name == "oc-admin" {
            return None;
        }
        if name == "help" {
            let mut names = vec!["help", "oc", "ut", "tut", "st", "test-notify"];
            names.extend(self.responses.keys().map(String::as_str));
            names.sort_unstable();
            return Some(format!(
                "{}\n\n使えるコマンド\n{}",
                self.help["index"],
                names
                    .into_iter()
                    .map(|name| format!("・!{name}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        self.help
            .get(name)
            .or_else(|| self.responses.get(name))
            .cloned()
    }
}

fn read_folder(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "txt")
        {
            continue;
        }
        if !entry.file_type()?.is_file() || entry.metadata()?.len() > 8192 {
            return Err("InvalidContentFile".into());
        }
        let name = entry
            .path()
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or("InvalidContentName")?
            .to_owned();
        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
        {
            return Err("InvalidContentName".into());
        }
        let text = std::fs::read_to_string(entry.path())?
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n")
            .trim()
            .to_owned();
        if text.is_empty() || text.encode_utf16().count() > 1500 || text.contains("```") {
            return Err("InvalidContentText".into());
        }
        result.insert(name, text);
        if result.len() > 128 {
            return Err("ContentCapacity".into());
        }
    }
    Ok(result)
}
