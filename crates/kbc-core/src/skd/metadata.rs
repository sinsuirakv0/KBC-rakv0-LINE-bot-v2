//! Discord版と同じ名前・シリーズ対応表を取得する。取得結果は処理中だけ保持する。

use super::model::*;
use crate::{Result, assets::AssetService, messages::Messages};
use std::{collections::HashMap, sync::Arc};

const EVENT: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main";
const ASSETS: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata";

pub(crate) struct Metadata {
    http: Arc<AssetService>,
    messages: Arc<Messages>,
}
impl Metadata {
    pub fn new(http: Arc<AssetService>, messages: Arc<Messages>) -> Self {
        Self { http, messages }
    }
    async fn text(&self, base: &str, path: &str) -> Result<String> {
        Ok(self.http.get_text(&format!("{base}/{path}")).await?)
    }
    pub async fn gatya(&self, gacha: GachaJson) -> Result<GachaScheduleData> {
        let mut names = Vec::new();
        let mut gacha_names = Vec::new();
        let mut mappings = Vec::new();
        for (name, short, mapping) in [
            (
                "gatya_name.csv",
                "gatya_series_name_omit.csv",
                "GatyaData_Option_SetR.tsv",
            ),
            (
                "gatya_e_name.csv",
                "gatya_e_series_name_omit.csv",
                "GatyaData_Option_SetE.tsv",
            ),
            (
                "gatya_n_name.csv",
                "gatya_n_series_nameomit.csv",
                "GatyaData_Option_SetN.tsv",
            ),
        ] {
            let name_path = format!("data/{name}");
            let mapping_path = format!("Data/{mapping}");
            let (full, mapping) = tokio::try_join!(
                self.text(EVENT, &name_path),
                self.text(ASSETS, &mapping_path)
            )?;
            let full = id_names(&full);
            let mapping = parse_mapping(&mapping)?;
            let mut ids: Vec<_> = mapping.iter().collect();
            ids.sort_by_key(|(id, _)| **id);
            let mut derived = HashMap::new();
            for (id, series) in ids {
                if let Some(name) = full.get(id) {
                    derived.entry(*series).or_insert_with(|| name.clone());
                }
            }
            // 略称は404のときだけ通常名へ戻し、通信失敗を空の辞書として扱わない。
            if let Some(short) = self
                .http
                .get_optional_text(&format!("{EVENT}/data/{short}"))
                .await?
            {
                derived.extend(id_names(&short));
            }
            gacha_names.push(full);
            names.push(derived);
            mappings.push(mapping);
        }
        let mut names = names.into_iter();
        let mut gacha_names = gacha_names.into_iter();
        let mut mappings = mappings.into_iter();
        Ok(GachaScheduleData {
            gacha,
            gacha_names: ModeMaps {
                rare: gacha_names.next().unwrap(),
                event: gacha_names.next().unwrap(),
                normal: gacha_names.next().unwrap(),
            },
            short_series_names: ModeMaps {
                rare: names.next().unwrap(),
                event: names.next().unwrap(),
                normal: names.next().unwrap(),
            },
            series_mappings: ModeMaps {
                rare: mappings.next().unwrap(),
                event: mappings.next().unwrap(),
                normal: mappings.next().unwrap(),
            },
            messages: Arc::clone(&self.messages),
        })
    }
    pub async fn sale(&self, sale: SaleJson) -> Result<SaleDisplayData> {
        let (sale_names, all_day, missions, cards) = tokio::try_join!(
            self.text(EVENT, "data/sale_name.csv"),
            self.text(ASSETS, "res/All_day_event.tsv"),
            self.text(ASSETS, "res/Mission_Name.csv"),
            self.text(EVENT, "setting/cardsetting")
        )?;
        let all_day_event_names = all_day
            .lines()
            .filter_map(|line| {
                let (name, cells) = line.split_once('\t')?;
                if name.trim().is_empty() || name.trim().starts_with("//") {
                    return None;
                }
                Some((
                    cells.split('\t').next()?.trim().parse().ok()?,
                    name.trim().to_owned(),
                ))
            })
            .collect();
        let mut card_setting_stage_ids = Vec::new();
        for line in cards.lines() {
            let line = line
                .split('#')
                .next()
                .unwrap_or("")
                .split("//")
                .next()
                .unwrap_or("");
            for token in line.split(|c: char| c == ',' || c.is_whitespace()) {
                if let Ok(id) = token.parse()
                    && !card_setting_stage_ids.contains(&id)
                {
                    card_setting_stage_ids.push(id);
                }
            }
        }
        Ok(SaleDisplayData {
            sale,
            sale_names: id_names(&sale_names),
            all_day_event_names,
            mission_names: id_names(&missions),
            card_setting_stage_ids,
            messages: Arc::clone(&self.messages),
        })
    }
    pub async fn item(&self, item: ItemJson) -> Result<ItemDisplayData> {
        let (names, sale) = tokio::try_join!(
            self.text(EVENT, "data/item_name.csv"),
            self.text(EVENT, "data/sale_name.csv")
        )?;
        let item_names = names
            .lines()
            .filter_map(|line| {
                let mut cells = line.trim_start_matches('\u{feff}').split(',');
                let id = cells.next()?.trim().parse().ok()?;
                let name = cells.next()?.trim();
                (!name.is_empty()).then(|| {
                    (
                        id,
                        ItemName {
                            name: name.into(),
                            detail: cells.collect::<Vec<_>>().join(",").trim().into(),
                        },
                    )
                })
            })
            .collect();
        Ok(ItemDisplayData {
            item,
            item_names,
            sale_names: id_names(&sale),
            messages: Arc::clone(&self.messages),
        })
    }
}

pub(crate) fn id_names(text: &str) -> HashMap<i64, String> {
    text.lines()
        .filter_map(|line| {
            let (id, name) = line.trim_start_matches('\u{feff}').split_once(',')?;
            (!name.trim().is_empty())
                .then(|| Some((id.trim().parse().ok()?, name.trim().to_owned())))?
        })
        .collect()
}
pub(crate) fn parse_mapping(text: &str) -> Result<HashMap<i64, i64>> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let headers: Vec<_> = lines
        .next()
        .unwrap_or("")
        .split('\t')
        .map(str::trim)
        .collect();
    let id = headers
        .iter()
        .position(|value| *value == "GatyaSetID")
        .ok_or("MissingGatyaSetId")?;
    let series = headers
        .iter()
        .position(|value| *value == "seriesID")
        .ok_or("MissingSeriesId")?;
    Ok(lines
        .filter_map(|line| {
            let cells: Vec<_> = line.split('\t').collect();
            Some((
                cells.get(id)?.trim().parse().ok()?,
                cells.get(series)?.trim().parse().ok()?,
            ))
        })
        .collect())
}
