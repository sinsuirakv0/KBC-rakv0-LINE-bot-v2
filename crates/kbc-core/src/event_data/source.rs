//! 最新の公開JSONを検証し、skdと同じ名称・日時処理へ接続する。
use crate::skd::{
    metadata::{Metadata, id_names, parse_mapping},
    model::*,
};
use crate::{Result, assets::AssetService, messages::Messages};
use std::{collections::HashMap, sync::Arc};

const BASE: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data";
pub(crate) struct Source {
    http: Arc<AssetService>,
    metadata: Metadata,
}
pub(super) struct GachaListData {
    pub gacha: GachaJson,
    pub item: ItemJson,
    pub sale_names: HashMap<i64, String>,
    pub short_series_names: NameMaps,
    pub series_mappings: MappingMaps,
}
pub(crate) struct GachaLookupData {
    pub gacha: GachaJson,
    pub gacha_names: NameMaps,
    pub series_names: NameMaps,
    pub short_series_names: NameMaps,
    pub series_mappings: MappingMaps,
}
pub(super) struct GachaJsonWithMappings {
    pub gacha: GachaJson,
    pub series_mappings: MappingMaps,
}
impl Source {
    pub fn new(http: Arc<AssetService>, messages: Arc<Messages>) -> Self {
        Self {
            metadata: Metadata::new(Arc::clone(&http), messages),
            http,
        }
    }
    pub async fn fetch_gacha_json(&self) -> Result<GachaJson> {
        let data: GachaJson = self.json("gatya").await?;
        for block in &data.data {
            validate_header(&block.header.schedule)?;
        }
        Ok(data)
    }
    pub async fn fetch_sale_json(&self) -> Result<SaleJson> {
        let data: SaleJson = self.json("sale").await?;
        for entry in &data.data {
            validate_header(&entry.header)?;
        }
        Ok(data)
    }
    pub async fn fetch_item_json(&self) -> Result<ItemJson> {
        let data: ItemJson = self.json("item").await?;
        for entry in &data.data {
            validate_header(&entry.header)?;
        }
        Ok(data)
    }
    async fn json<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T> {
        let text = self.http.get_text(&format!("{BASE}/{name}.json")).await?;
        Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
    }
    pub(super) async fn fetch_json_with_mappings(&self) -> Result<GachaJsonWithMappings> {
        let mapping = |mode| async move {
            let text = self.http.get_text(&format!("https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/Data/GatyaData_Option_Set{mode}.tsv")).await?;
            parse_mapping(&text)
        };
        let (gacha, rare, event, normal) = tokio::try_join!(
            self.fetch_gacha_json(),
            mapping("R"),
            mapping("E"),
            mapping("N")
        )?;
        Ok(GachaJsonWithMappings {
            gacha,
            series_mappings: ModeMaps {
                rare,
                event,
                normal,
            },
        })
    }
    pub(super) async fn fetch_schedule_data(&self) -> Result<GachaListData> {
        let (data, item, names) = tokio::try_join!(
            self.metadata.gatya(self.fetch_gacha_json().await?),
            self.fetch_item_json(),
            async {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                    self.http.get_text(&format!("{BASE}/sale_name.csv")).await?,
                )
            }
        )?;
        Ok(GachaListData {
            gacha: data.gacha,
            item,
            sale_names: id_names(&names),
            short_series_names: data.short_series_names,
            series_mappings: data.series_mappings,
        })
    }
    pub async fn fetch_lookup_data(&self) -> Result<GachaLookupData> {
        let data = self.metadata.gatya(self.fetch_gacha_json().await?).await?;
        let names = |path: &'static str| async move {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                self.http
                    .get_optional_text(&format!("{BASE}/{path}"))
                    .await?
                    .map(|text| id_names(&text))
                    .unwrap_or_default(),
            )
        };
        let (rare, event, normal) = tokio::try_join!(
            names("gatya_series_name.csv"),
            names("gatya_e_series_name.csv"),
            names("gatya_n_series_name.csv")
        )?;
        Ok(GachaLookupData {
            gacha: data.gacha,
            gacha_names: data.gacha_names,
            series_names: ModeMaps {
                rare,
                event,
                normal,
            },
            short_series_names: data.short_series_names,
            series_mappings: data.series_mappings,
        })
    }
    pub async fn sale(&self) -> Result<SaleDisplayData> {
        self.metadata.sale(self.fetch_sale_json().await?).await
    }
    pub async fn item(&self) -> Result<ItemDisplayData> {
        self.metadata.item(self.fetch_item_json().await?).await
    }
}
