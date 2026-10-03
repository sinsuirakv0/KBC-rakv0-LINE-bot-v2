use crate::Result;
use serde::Deserialize;
use std::{collections::HashMap, path::Path};

#[derive(Default)]
pub struct Permissions(HashMap<(String, String), String>);
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grant {
    chat_mid: String,
    user_mid: String,
    chat_type: String,
    role: String,
}
#[derive(Deserialize)]
struct File {
    version: u32,
    roles: Vec<Grant>,
}
impl Permissions {
    // 旧permissions.jsonの権限だけを読む。実アカウントのMIDをコードへ埋め込まない。
    pub fn load(path: Option<&str>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        if std::fs::metadata(Path::new(path))?.len() > 256 * 1024 {
            return Err("PermissionFileLimit".into());
        }
        let text = std::fs::read_to_string(path)?;
        let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        if file.version != 1 || file.roles.len() > 4096 {
            return Err("InvalidPermissionFile".into());
        }
        let mut roles = HashMap::new();
        for grant in file.roles {
            if grant.chat_type != "SQUARE" {
                continue;
            }
            if grant.chat_mid.is_empty()
                || grant.chat_mid.len() > 256
                || !grant.user_mid.starts_with('p')
                || grant.user_mid.len() > 256
                || !matches!(grant.role.as_str(), "admin" | "mod")
            {
                return Err("InvalidPermissionGrant".into());
            }
            let key = (grant.chat_mid, grant.user_mid);
            if roles.get(&key).is_none_or(|role| role != "admin") {
                roles.insert(key, grant.role);
            }
        }
        Ok(Self(roles))
    }
    pub fn rank(&self, square: &str, member: &str) -> u8 {
        match self
            .0
            .get(&(square.into(), member.into()))
            .map(String::as_str)
        {
            Some("admin") => 2,
            Some("mod") => 1,
            _ => 0,
        }
    }
}
