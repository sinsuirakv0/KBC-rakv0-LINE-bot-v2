//! Schedule履歴から指定更新を選び、追加・変更内容を構築する。

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::metadata::Metadata;
use crate::assets::AssetService;
use crate::messages::Messages;

use super::diff::{ScheduleChanges, compare_gatya, compare_item, compare_sale};
use super::formatter::{AddedScheduleData, format_added_schedules, history_url};
use super::parser::{parse_gatya_tsv, parse_item_tsv, parse_sale_tsv};

const SOURCE_BASE_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event";
const SOURCE_TREE_URL: &str = "https://api.github.com/repos/sinsuirakv0/KBC-rakv0-event/git/trees";
const SOURCE_REVISION_URL: &str =
    "https://api.github.com/repos/sinsuirakv0/KBC-rakv0-event/git/ref/heads/main";
const HISTORY_GROUP_SECONDS: i64 = 100;
const MAX_HISTORY_FILES: usize = 100_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ScheduleType {
    Gatya,
    Sale,
    Item,
}

impl ScheduleType {
    pub(crate) const ALL: [Self; 3] = [Self::Gatya, Self::Sale, Self::Item];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Gatya => "gatya",
            Self::Sale => "sale",
            Self::Item => "item",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "gatya" => Some(Self::Gatya),
            "sale" => Some(Self::Sale),
            "item" => Some(Self::Item),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct HistoryFile {
    data_type: ScheduleType,
    path: String,
    timestamp: i64,
}

struct ScheduleUpdate {
    timestamp: i64,
    files: Vec<HistoryFile>,
}

struct FileReference {
    revision: String,
    path: String,
}

struct Comparison {
    data_type: ScheduleType,
    before: Option<FileReference>,
    after: FileReference,
}

pub(super) struct SkdUpdate {
    pub(super) timestamp: DateTime<Utc>,
    pub(super) cursor: i64,
    pub(super) types: Vec<ScheduleType>,
    pub(super) initial_types: Vec<ScheduleType>,
    pub(super) contents: Vec<String>,
}

pub(crate) struct SkdDataSource {
    http: Arc<AssetService>,
    metadata: Metadata,
    messages: Arc<Messages>,
}

impl SkdDataSource {
    pub(crate) fn new(http: Arc<AssetService>, messages: Arc<Messages>) -> Self {
        Self {
            metadata: Metadata::new(Arc::clone(&http), Arc::clone(&messages)),
            messages,
            http,
        }
    }

    pub(super) async fn load(
        &self,
        date: Option<NaiveDate>,
    ) -> Result<Option<SkdUpdate>, SkdDataError> {
        let (revision, files) = self.read_history_snapshot(None).await?;
        let Some(update) = select_update(&files, date) else {
            return Ok(None);
        };
        self.build_update(&revision, &files, update).await.map(Some)
    }

    pub(super) async fn latest_timestamp(
        &self,
        hashes: &BTreeMap<String, String>,
    ) -> Result<i64, SkdDataError> {
        let (_, files) = self.read_history_snapshot(Some(hashes)).await?;
        select_update(&files, None)
            .and_then(|update| update.files.iter().map(|file| file.timestamp).max())
            .ok_or_else(|| SkdDataError::new("schedule history is empty"))
    }

    pub(super) async fn updates_since(
        &self,
        timestamp: i64,
        hashes: &BTreeMap<String, String>,
    ) -> Result<(Vec<SkdUpdate>, bool), SkdDataError> {
        let (revision, files) = self.read_history_snapshot(Some(hashes)).await?;
        let mut updates = updates_after(&files, timestamp);
        let more = updates.len() > 4;
        updates.truncate(4);
        let mut results = Vec::new();
        for update in updates {
            results.push(self.build_update(&revision, &files, update).await?);
        }
        Ok((results, more))
    }

    async fn build_update(
        &self,
        revision: &str,
        files: &[HistoryFile],
        update: ScheduleUpdate,
    ) -> Result<SkdUpdate, SkdDataError> {
        let mut comparisons = Vec::new();
        for data_type in ScheduleType::ALL {
            let Some(after) = update
                .files
                .iter()
                .filter(|file| file.data_type == data_type)
                .max_by_key(|file| file.timestamp)
            else {
                continue;
            };
            let before = files
                .iter()
                .filter(|file| file.data_type == data_type && file.timestamp < update.timestamp)
                .max_by_key(|file| file.timestamp)
                .map(|file| FileReference {
                    revision: revision.to_owned(),
                    path: file.path.clone(),
                });
            comparisons.push(Comparison {
                data_type,
                before,
                after: FileReference {
                    revision: revision.to_owned(),
                    path: after.path.clone(),
                },
            });
        }
        let timestamp = Utc
            .timestamp_opt(update.timestamp, 0)
            .single()
            .ok_or_else(|| SkdDataError::new("schedule timestamp is invalid"))?;
        let initial_types = comparisons
            .iter()
            .filter(|comparison| comparison.before.is_none())
            .map(|comparison| comparison.data_type)
            .collect::<Vec<_>>();
        let types = comparisons
            .iter()
            .map(|comparison| comparison.data_type)
            .collect::<Vec<_>>();
        let paths = comparisons
            .iter()
            .map(|comparison| comparison.after.path.clone())
            .collect::<Vec<_>>();
        let data = self.build_data(comparisons).await?;
        let contents =
            format_added_schedules(&data, timestamp, &history_url(&paths), &self.messages)
                .map_err(SkdDataError::new)?;
        Ok(SkdUpdate {
            timestamp,
            cursor: update
                .files
                .iter()
                .map(|file| file.timestamp)
                .max()
                .unwrap_or(update.timestamp),
            types,
            initial_types,
            contents,
        })
    }

    async fn read_history_snapshot(
        &self,
        hashes: Option<&BTreeMap<String, String>>,
    ) -> Result<(String, Vec<HistoryFile>), SkdDataError> {
        let revision: RevisionResponse = self.read_json(SOURCE_REVISION_URL).await?;
        let revision = revision
            .object
            .and_then(|object| object.sha)
            .filter(|value| valid_revision(value))
            .ok_or_else(|| SkdDataError::new("schedule revision response is invalid"))?;
        if let Some(hashes) = hashes {
            let text = self
                .read_text(&format!(
                    "{SOURCE_BASE_URL}/{revision}/state/skd-notifications.json"
                ))
                .await?;
            let confirmed = super::monitor::parse_hashes(&text)
                .map_err(|error| SkdDataError::new(error.to_string()))?;
            if &confirmed != hashes {
                return Err(SkdDataError::new("schedule source changed"));
            }
        }
        let files = self.read_history_at(&revision).await?;
        Ok((revision, files))
    }

    async fn read_history_at(&self, revision: &str) -> Result<Vec<HistoryFile>, SkdDataError> {
        if !valid_revision(revision) {
            return Err(SkdDataError::new("schedule revision is invalid"));
        }
        let tree: TreeResponse = self
            .read_json(&format!("{SOURCE_TREE_URL}/{revision}?recursive=1"))
            .await?;
        if tree.truncated != Some(false) || tree.tree.len() > MAX_HISTORY_FILES {
            return Err(SkdDataError::new("schedule history tree is invalid"));
        }
        let mut files = Vec::new();
        for entry in tree.tree {
            if entry.entry_type != "blob" {
                continue;
            }
            if let Some(file) = parse_history_path(&entry.path) {
                files.push(file);
            }
        }
        Ok(files)
    }

    async fn build_data(
        &self,
        comparisons: Vec<Comparison>,
    ) -> Result<AddedScheduleData, SkdDataError> {
        let mut data = AddedScheduleData::default();
        let mut changes = ScheduleChanges::default();
        for comparison in comparisons {
            let before_request = async {
                match comparison.before.as_ref() {
                    Some(file) => self.read_file(file, comparison.data_type).await,
                    None => Ok(String::new()),
                }
            };
            let (before, after) = tokio::try_join!(
                before_request,
                self.read_file(&comparison.after, comparison.data_type)
            )?;
            match comparison.data_type {
                ScheduleType::Gatya => {
                    let before = parse_gatya_tsv(&before).map_err(SkdDataError::new)?;
                    let after_parsed = parse_gatya_tsv(&after).map_err(SkdDataError::new)?;
                    if after_parsed.data.is_empty()
                        && after.chars().any(|value| value.is_ascii_digit())
                    {
                        return Err(SkdDataError::new("gatya TSV could not be parsed"));
                    }
                    let (added, changed) =
                        compare_gatya(before, after_parsed).map_err(SkdDataError::new)?;
                    changes.gatya = changed;
                    data.gatya = Some(
                        self.metadata
                            .gatya(added)
                            .await
                            .map_err(|error| SkdDataError::new(error.to_string()))?,
                    );
                }
                ScheduleType::Sale => {
                    let before = parse_sale_tsv(&before).map_err(SkdDataError::new)?;
                    let after = parse_sale_tsv(&after).map_err(SkdDataError::new)?;
                    let (added, changed) =
                        compare_sale(before, after).map_err(SkdDataError::new)?;
                    changes.sale = changed;
                    data.sale = Some(
                        self.metadata
                            .sale(added)
                            .await
                            .map_err(|error| SkdDataError::new(error.to_string()))?,
                    );
                }
                ScheduleType::Item => {
                    let before = parse_item_tsv(&before).map_err(SkdDataError::new)?;
                    let after = parse_item_tsv(&after).map_err(SkdDataError::new)?;
                    let (added, changed) =
                        compare_item(before, after).map_err(SkdDataError::new)?;
                    changes.item = changed;
                    data.item = Some(
                        self.metadata
                            .item(added)
                            .await
                            .map_err(|error| SkdDataError::new(error.to_string()))?,
                    );
                }
            }
        }
        data.changes = changes;
        Ok(data)
    }

    async fn read_file(
        &self,
        file: &FileReference,
        data_type: ScheduleType,
    ) -> Result<String, SkdDataError> {
        if !valid_revision(&file.revision)
            || parse_history_path(&file.path).is_none_or(|parsed| parsed.data_type != data_type)
        {
            return Err(SkdDataError::new("schedule file reference is invalid"));
        }
        let text = self
            .read_text(&format!(
                "{SOURCE_BASE_URL}/{}/{}",
                file.revision, file.path
            ))
            .await?;
        Ok(text)
    }

    async fn read_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, SkdDataError> {
        let text = self.read_text(url).await?;
        serde_json::from_str(&text)
            .map_err(|error| SkdDataError::new(format!("schedule JSON is invalid: {error}")))
    }

    async fn read_text(&self, url: &str) -> Result<String, SkdDataError> {
        let text = self
            .http
            .get_text(url)
            .await
            .map_err(|error| SkdDataError::new(error.to_string()))?;
        if text.trim().is_empty() {
            return Err(SkdDataError::new("schedule document is empty"));
        }
        Ok(text)
    }
}

fn group_updates(files: &[HistoryFile]) -> Vec<ScheduleUpdate> {
    let mut sorted = files.to_vec();
    sorted.sort_by_key(|file| file.timestamp);
    let mut updates: Vec<ScheduleUpdate> = Vec::new();
    for file in sorted {
        if let Some(previous) = updates.last_mut()
            && file.timestamp - previous.timestamp <= HISTORY_GROUP_SECONDS
        {
            previous.files.push(file);
        } else {
            updates.push(ScheduleUpdate {
                timestamp: file.timestamp,
                files: vec![file],
            });
        }
    }
    updates
}

fn select_update(files: &[HistoryFile], date: Option<NaiveDate>) -> Option<ScheduleUpdate> {
    let mut updates = group_updates(files);
    let Some(target) = date else {
        return updates.pop();
    };
    updates.into_iter().min_by(|left, right| {
        let left_date = update_date(left.timestamp);
        let right_date = update_date(right.timestamp);
        let left_distance = left_date
            .map(|value| (value - target).num_days().unsigned_abs())
            .unwrap_or(u64::MAX);
        let right_distance = right_date
            .map(|value| (value - target).num_days().unsigned_abs())
            .unwrap_or(u64::MAX);
        left_distance
            .cmp(&right_distance)
            .then_with(|| left_date.cmp(&right_date))
            .then_with(|| right.timestamp.cmp(&left.timestamp))
    })
}

fn updates_after(files: &[HistoryFile], timestamp: i64) -> Vec<ScheduleUpdate> {
    let mut updates = group_updates(files);
    // 同じ100秒の組に後から保存された種別も拾う。既に通知したファイルは再比較しない。
    for update in &mut updates {
        update.files.retain(|file| file.timestamp > timestamp);
        if let Some(first) = update.files.first() {
            update.timestamp = first.timestamp;
        }
    }
    updates.retain(|update| !update.files.is_empty());
    updates
}

fn update_date(timestamp: i64) -> Option<NaiveDate> {
    let timezone = FixedOffset::east_opt(9 * 60 * 60)?;
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.with_timezone(&timezone).date_naive())
}

fn parse_history_path(path: &str) -> Option<HistoryFile> {
    let name = path.strip_prefix("raw/")?.strip_suffix(".tsv")?;
    let (type_name, timestamp) = name.rsplit_once('_')?;
    let data_type = ScheduleType::parse(type_name)?;
    let timestamp = timestamp.parse().ok()?;
    Utc.timestamp_opt(timestamp, 0).single()?;
    Some(HistoryFile {
        data_type,
        path: path.to_owned(),
        timestamp,
    })
}

fn valid_revision(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

#[derive(Deserialize)]
struct RevisionResponse {
    object: Option<RevisionObject>,
}

#[derive(Deserialize)]
struct RevisionObject {
    sha: Option<String>,
}

#[derive(Deserialize)]
struct TreeResponse {
    truncated: Option<bool>,
    #[serde(default)]
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug)]
pub(crate) struct SkdDataError {
    reason: String,
}

impl SkdDataError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Display for SkdDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}
impl std::error::Error for SkdDataError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picks_nearest_day_and_tracks_late_files_in_same_group() {
        let first = Utc
            .with_ymd_and_hms(2026, 10, 1, 0, 0, 0)
            .unwrap()
            .timestamp();
        let files = vec![
            parse_history_path(&format!("raw/gatya_{first}.tsv")).unwrap(),
            parse_history_path(&format!("raw/sale_{}.tsv", first + 60)).unwrap(),
            parse_history_path(&format!("raw/item_{}.tsv", first + 3600)).unwrap(),
        ];
        assert_eq!(
            select_update(&files, Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()))
                .unwrap()
                .timestamp,
            first + 3600
        );
        let updates = updates_after(&files, first);
        assert_eq!(updates[0].timestamp, first + 60);
        assert_eq!(updates[0].files[0].data_type, ScheduleType::Sale);
        assert_eq!(updates.len(), 2);
        assert!(updates_after(&files, first + 3600).is_empty());
        assert!(parse_history_path("raw/../sale_123.tsv").is_none());
    }
}
