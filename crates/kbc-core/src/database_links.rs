use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub(crate) enum DatabaseSite {
    #[default]
    Kbc,
    Jdb,
}

impl DatabaseSite {
    pub(crate) fn split_arguments<T: AsRef<str>>(arguments: &[T]) -> (&[T], Self) {
        if arguments
            .last()
            .is_some_and(|value| value.as_ref().eq_ignore_ascii_case("jdb"))
        {
            (&arguments[..arguments.len() - 1], Self::Jdb)
        } else {
            (arguments, Self::Kbc)
        }
    }

    pub(crate) fn search_url(self, kind: &str) -> String {
        let page = match kind {
            "ut" => "unit",
            "tut" => "tunit",
            _ => "stage",
        };
        match self {
            Self::Kbc => format!("https://kbc-rakv0.pages.dev/pages/{page}-search/"),
            Self::Jdb => format!(
                "https://jarjarblink.github.io/JDB/{}_search.html?cc=ja",
                if kind == "st" { "map" } else { page }
            ),
        }
    }

    pub(crate) fn detail_url(self, kind: &str, id: &str, jdb_url: String) -> String {
        if matches!(self, Self::Jdb) {
            return jdb_url;
        }
        let query = match kind {
            "ut" => format!("unit-detail/?id={id}"),
            "tut" => format!("tunit-detail/?id={id}"),
            _ => {
                let (map, stage) = id
                    .split_once('-')
                    .map_or((id, None), |(map, stage)| (map, Some(stage)));
                let key = stage_map_key(map);
                let mut query = format!("stage-detail/?map={key}");
                if let Some(stage) = stage {
                    let value = stage.trim_start_matches('0');
                    query.push_str(&format!(
                        "&stage={}",
                        if value.is_empty() { "0" } else { value }
                    ));
                }
                query
            }
        };
        format!("https://kbc-rakv0.pages.dev/pages/{query}")
    }
}

// 既存検索IDの章・EX・侵略表記をKBCのマップキーへ対応させる。
fn stage_map_key(id: &str) -> String {
    match id.parse::<usize>().ok() {
        Some(value @ 3000..=3002) => return format!("EOC{}", value - 2999),
        Some(value @ 3003..=3005) => return format!("ITF{}", value - 3002),
        Some(value @ 3006..=3008) => return format!("COTC{}", value - 3005),
        Some(value @ 20000..=20002) => return format!("EOCZ{}", value - 19999),
        Some(value @ 21000..=21002) => return format!("ITFZ{}", value - 20999),
        Some(value @ 22000..=22002) => return format!("COTCZ{}", value - 21999),
        _ => {}
    }
    match id {
        "2_Inv000" => "COTC-I".into(),
        "2Z_Inv000" => "COTCZ-I".into(),
        _ if id.strip_prefix('E').is_some_and(|tail| {
            !tail.is_empty() && tail.bytes().all(|value| value.is_ascii_digit())
        }) =>
        {
            format!("EX{}", &id[1..])
        }
        _ => id.into(),
    }
}
