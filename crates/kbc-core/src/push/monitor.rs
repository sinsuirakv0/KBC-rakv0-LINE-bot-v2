use super::{insert, kind_label, notification, schedule, time};
use crate::{
    Result, Runtime,
    event_data::source::{GachaLookupData, Source},
    messages::{Messages, message},
    now_ms,
    skd::{labels::*, model::*},
};
use chrono::{DateTime, Duration, Timelike, Utc};
use rusqlite::{Transaction, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

struct Catalog {
    sale: Option<SaleDisplayData>,
    gacha: Option<GachaLookupData>,
    version: i64,
}
struct Subscription {
    kind: String,
    id: i64,
    advance: i64,
    since: i64,
}
struct Notice {
    due: i64,
    phase: String,
    text: String,
    contents: Option<Vec<String>>,
}

impl Runtime {
    // キャッシュは取得後120秒まで。HTTP・配送・保存は既存の共通枠を使う。
    pub(crate) async fn push_loop(&self) {
        let source = Source::new(
            Arc::new(self.assets.clone()),
            Arc::clone(&self.content.messages),
        );
        let mut catalog = None;
        let mut fetched = 0;
        let mut next_fetch = 0;
        loop {
            let now = now_ms();
            if now - fetched >= 120_000 {
                catalog = None;
            }
            let wanted = self.push_topics();
            match wanted {
                Ok((false, false)) => {
                    catalog = None;
                    next_fetch = 0;
                }
                Ok((sale, gacha)) => {
                    if catalog.as_ref().is_some_and(|c: &Catalog| {
                        (sale && c.sale.is_none()) || (gacha && c.gacha.is_none())
                    }) {
                        catalog = None;
                        next_fetch = 0;
                    }
                    if now >= next_fetch {
                        let fetch = async {
                            let (sale, gacha, version) = tokio::try_join!(
                                async {
                                    if sale {
                                        source.sale().await.map(Some)
                                    } else {
                                        Ok(None)
                                    }
                                },
                                async {
                                    if gacha {
                                        source.fetch_lookup_data().await.map(Some)
                                    } else {
                                        Ok(None)
                                    }
                                },
                                async {
                                    Ok::<_,Box<dyn std::error::Error+Send+Sync>>(self.assets.get_text("https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/setting/version").await?.trim().parse::<i64>()?)
                                }
                            )?;
                            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Catalog {
                                sale,
                                gacha,
                                version,
                            })
                        };
                        match tokio::select! {_=self.cancel.cancelled()=>return,result=fetch=>result}
                        {
                            Ok(data) => {
                                catalog = Some(data);
                                fetched = now_ms();
                                next_fetch = fetched + 60_000;
                            }
                            Err(error) => {
                                eprintln!("Push source: {error}");
                                next_fetch = now_ms() + 60_000;
                            }
                        }
                    }
                    if now_ms() - fetched < 120_000
                        && let Some(data) = &catalog
                        && let Err(error) = self.dispatch_push(data, now_ms())
                    {
                        eprintln!("Push dispatch: {error}");
                    }
                }
                Err(error) => eprintln!("Push settings: {error}"),
            }
            tokio::select! {_=self.cancel.cancelled()=>return,_=tokio::time::sleep(std::time::Duration::from_secs(5))=>{}}
        }
    }
    fn push_topics(&self) -> Result<(bool, bool)> {
        let db = self.database.lock().map_err(|_| "DatabaseLock")?;
        Ok((db.query_row("SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE kind IN ('sale','all','daily'))",[],|r|r.get(0))?,db.query_row("SELECT EXISTS(SELECT 1 FROM push_subscriptions WHERE kind IN ('g','gr','ge','gn','g-all'))",[],|r|r.get(0))?))
    }
    fn dispatch_push(&self, catalog: &Catalog, now: i64) -> Result<()> {
        let mut db = self.database.lock().map_err(|_| "DatabaseLock")?;
        let tx = db.transaction()?;
        // 各トークの確定済み走査位置より古い印だけを消す。滞留トークの印は保持する。
        tx.execute("DELETE FROM push_deliveries WHERE due<?1 AND due<COALESCE((SELECT checked FROM push_destinations d WHERE d.chat=push_deliveries.chat),?1)",[now-120_000])?;
        let chats=tx.prepare("SELECT chat,checked FROM push_destinations WHERE checked<?1 ORDER BY checked,chat LIMIT 64")?
            .query_map([now+30_000],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut remaining = 64;
        for (chat, from) in chats {
            let to = (now + 60_000).min(from + 300_000);
            if crate::permissions::stopped(&tx, &chat)? {
                tx.execute(
                    "UPDATE push_destinations SET checked=?2 WHERE chat=?1",
                    params![chat, now],
                )?;
                continue;
            }
            let settings = tx
                .prepare("SELECT kind,target,advance,since FROM push_subscriptions WHERE chat=?1")?
                .query_map([&chat], |r| {
                    Ok(Subscription {
                        kind: r.get(0)?,
                        id: r.get(1)?,
                        advance: r.get(2)?,
                        since: r.get(3)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let notices = collect(catalog, &settings, &self.content.messages, from, to)?;
            let mut completed = true;
            for notice in notices {
                if !save(&tx, &chat, notice, now, &mut remaining)? {
                    completed = false;
                    break;
                }
            }
            if completed {
                tx.execute(
                    "UPDATE push_destinations SET checked=?2 WHERE chat=?1",
                    params![chat, to],
                )?;
            }
            if remaining == 0 {
                break;
            }
        }
        tx.commit()?;
        self.wake.notify_waiters();
        Ok(())
    }
}

fn save(
    tx: &Transaction<'_>,
    chat: &str,
    notice: Notice,
    now: i64,
    remaining: &mut usize,
) -> Result<bool> {
    use sha1::{Digest, Sha1};
    let id = format!(
        "push:event:{:x}:{}:{}",
        Sha1::digest(chat.as_bytes()),
        notice.due,
        notice.phase
    );
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM push_deliveries WHERE id=?1)",
        [&id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(true);
    }
    let bodies = if notice.contents.is_some() {
        vec![notice.text]
    } else {
        crate::commands::split_text(&notice.text)
    };
    if bodies.is_empty() || bodies.iter().any(|body| body.encode_utf16().count() > 1500) {
        return Err("PushMessageLimit".into());
    }
    let count = bodies.len() + notice.contents.as_ref().map_or(0, Vec::len);
    if bodies.len() > crate::skd::MAX_THREAD_MESSAGES || count > crate::skd::MAX_THREAD_MESSAGES + 1
    {
        return Err("PushMessageLimit".into());
    }
    if bodies.len() > *remaining
        || crate::skd::pending_count(tx)? + count as i64 > crate::MAX_ACTIONS
    {
        return Ok(false);
    }
    if tx.query_row("SELECT count(*) FROM push_deliveries", [], |r| {
        r.get::<_, i64>(0)
    })? >= 32768
    {
        return Err("PushDeliveryCapacity".into());
    }
    for (index, text) in bodies.into_iter().enumerate() {
        let mut action = notification(id.clone(), chat, text, now);
        if let kbc_protocol::CoreAction::SendMessage {
            action_id,
            thread_contents,
            ..
        } = &mut action
        {
            *action_id = format!("{id}:{index}");
            if index == 0 {
                *thread_contents = notice.contents.clone();
            }
        }
        insert(tx, &action, notice.due)?;
        *remaining -= 1;
    }
    tx.execute(
        "INSERT INTO push_deliveries VALUES(?1,?2,?3)",
        params![id, chat, notice.due],
    )?;
    Ok(true)
}

fn available(header: &ScheduleHeader, version: i64) -> bool {
    header
        .max_version
        .parse::<i64>()
        .map_or(true, |max| max >= version)
}

fn collect(
    catalog: &Catalog,
    settings: &[Subscription],
    messages: &Messages,
    from: i64,
    to: i64,
) -> Result<Vec<Notice>> {
    let date = |value| DateTime::from_timestamp_millis(value).ok_or("InvalidPushTime");
    let mut groups: BTreeMap<(i64, String), BTreeSet<String>> = BTreeMap::new();
    let mut add =
        |setting: &Subscription, start: i64, end: i64, id: i64, name: String| -> Result<()> {
            let mut phases = vec![(
                start,
                "start".to_owned(),
                message!(
                    messages,
                    "push.started",
                    id = id,
                    name = name,
                    date = time::format_time(start),
                    end = time::format_time(end)
                ),
            )];
            if setting.kind != "all" && end - 600_000 >= start {
                phases.push((
                    end - 600_000,
                    "end".into(),
                    message!(
                        messages,
                        "push.ending",
                        id = id,
                        name = name,
                        end = time::format_time(end)
                    ),
                ));
            }
            if setting.advance > 0 {
                phases.push((
                    start - setting.advance * 60_000,
                    format!("before:{}", setting.advance),
                    message!(
                        messages,
                        "push.before",
                        id = id,
                        name = name,
                        minutes = setting.advance,
                        date = time::format_time(start)
                    ),
                ));
            }
            for (due, phase, line) in phases {
                if due > from && due <= to && due >= setting.since {
                    groups.entry((due, phase)).or_default().insert(line);
                    if groups.values().map(BTreeSet::len).sum::<usize>() > 4096 {
                        return Err("PushNoticeLimit".into());
                    }
                }
            }
            Ok(())
        };
    for setting in settings {
        if matches!(setting.kind.as_str(), "sale" | "all")
            && let Some(data) = &catalog.sale
        {
            for entry in &data.sale.data {
                if !available(&entry.header, catalog.version)
                    || (setting.kind == "sale" && !entry.stage_ids.contains(&setting.id))
                {
                    continue;
                }
                let periods = schedule::periods(entry, date(from)?, date(to)?)?;
                let mut periods = periods.into_iter().collect::<BTreeSet<_>>();
                if setting.advance > 0 {
                    periods.extend(schedule::periods(
                        entry,
                        date(from + setting.advance * 60_000)?,
                        date(to + setting.advance * 60_000)?,
                    )?);
                }
                let ids = if setting.kind == "sale" {
                    vec![setting.id]
                } else {
                    let priority = entry
                        .stage_ids
                        .iter()
                        .filter(|id| matches!(**id, 102 | 112))
                        .copied()
                        .collect::<Vec<_>>();
                    if priority.is_empty() {
                        entry
                            .stage_ids
                            .iter()
                            .copied()
                            .filter(|id| schedule::all_event(*id))
                            .collect()
                    } else {
                        priority
                    }
                };
                for (start, end) in periods {
                    for id in &ids {
                        add(setting, start, end, *id, stage_name(*id, data))?;
                    }
                }
            }
        } else if let Some(data) = &catalog.gacha
            && setting.kind.starts_with('g')
        {
            for block in &data.gacha.data {
                if !available(&block.header.schedule, catalog.version) {
                    continue;
                }
                let mode = mode_for_type(block.header.gacha_type);
                let kind = match mode {
                    GachaMode::Rare => "gr",
                    GachaMode::Event => "ge",
                    GachaMode::Normal => "gn",
                };
                let start = parse_header_date(
                    &block.header.schedule.start_date,
                    &block.header.schedule.start_time,
                )?
                .timestamp_millis();
                let end = parse_header_date(
                    &block.header.schedule.end_date,
                    &block.header.schedule.end_time,
                )?
                .timestamp_millis();
                for entry in &block.gachas {
                    if setting.kind != "g-all"
                        && !(entry.id == setting.id
                            && (setting.kind == "g" || setting.kind == kind))
                    {
                        continue;
                    }
                    let name = data
                        .gacha_names
                        .get(mode)
                        .get(&entry.id)
                        .cloned()
                        .unwrap_or_else(|| message!(messages, "skd.unknown").into());
                    add(
                        setting,
                        start,
                        end,
                        entry.id,
                        format!(
                            "{} {}",
                            kind_label(messages, kind),
                            strip_display_markup(&name, " ")
                        ),
                    )?;
                }
            }
        }
    }
    let mut result = groups
        .into_iter()
        .map(|((due, phase), lines)| Notice {
            due,
            phase,
            text: lines.into_iter().collect::<Vec<_>>().join("\n\n"),
            contents: None,
        })
        .collect::<Vec<_>>();
    if let Some(setting) = settings.iter().find(|s| s.kind == "daily")
        && let Some(data) = &catalog.sale
    {
        let first = date(from)?.with_timezone(&time::jst()).date_naive();
        let last = date(to)?.with_timezone(&time::jst()).date_naive();
        let mut day = first;
        while day <= last {
            let due =
                parse_header_date(&day.format("%Y%m%d").to_string(), "2200")?.timestamp_millis();
            if due > from && due <= to && due >= setting.since {
                let start = parse_header_date(
                    &(day + Duration::days(1)).format("%Y%m%d").to_string(),
                    "0000",
                )?;
                let end = start + Duration::days(1);
                let text = daily(data, catalog.version, messages, start, end)?;
                let contents = crate::commands::split_text(&text);
                if contents.len() > crate::skd::MAX_THREAD_MESSAGES {
                    return Err("PushDailyLimit".into());
                }
                result.push(Notice {
                    due,
                    phase: "daily".into(),
                    text: message!(
                        messages,
                        "push.daily_root",
                        date = time::format_time(start.timestamp_millis())
                    ),
                    contents: Some(contents),
                });
            }
            day += Duration::days(1);
        }
    }
    result.sort_by(|a, b| (a.due, &a.phase).cmp(&(b.due, &b.phase)));
    Ok(result)
}

fn daily(
    data: &SaleDisplayData,
    version: i64,
    messages: &Messages,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<String> {
    let mut groups: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for entry in &data.sale.data {
        if !available(&entry.header, version)
            || (entry.header.end_date == "20300101" && entry.time_blocks.is_empty())
        {
            continue;
        }
        let priority = entry
            .stage_ids
            .iter()
            .filter(|id| matches!(**id, 102 | 112))
            .copied()
            .collect::<Vec<_>>();
        let ids = if priority.is_empty() {
            entry
                .stage_ids
                .iter()
                .copied()
                .filter(|id| schedule::all_event(*id))
                .collect()
        } else {
            priority
        };
        for (a, b) in schedule::periods(entry, start, end)? {
            let (time, suffix) = if a >= start.timestamp_millis() && a < end.timestamp_millis() {
                (
                    a,
                    format_duration(
                        DateTime::from_timestamp_millis(a).unwrap(),
                        DateTime::from_timestamp_millis(b).unwrap(),
                    ),
                )
            } else if a < start.timestamp_millis()
                && b > start.timestamp_millis()
                && b < end.timestamp_millis()
            {
                let time = DateTime::from_timestamp_millis(b)
                    .unwrap()
                    .with_timezone(&time::jst());
                (
                    start.timestamp_millis(),
                    format!("~{:02}:{:02}", time.hour(), time.minute()),
                )
            } else {
                continue;
            };
            for id in &ids {
                groups
                    .entry(time)
                    .or_default()
                    .insert(format!("{id} {} {suffix}", stage_name(*id, data)));
            }
            if groups.values().map(BTreeSet::len).sum::<usize>() > 4096 {
                return Err("PushDailyLimit".into());
            }
        }
    }
    if groups.is_empty() {
        return Ok(message!(messages, "push.no_schedule").into());
    }
    Ok(groups
        .into_iter()
        .map(|(time, lines)| {
            let time = DateTime::from_timestamp_millis(time)
                .unwrap()
                .with_timezone(&time::jst());
            format!(
                "[{:02}:{:02}]\n{}",
                time.hour(),
                time.minute(),
                lines.into_iter().collect::<Vec<_>>().join("\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 公開データの実形を一度確認する用途。通常のテストでは通信しない。
    #[tokio::test]
    #[ignore = "公開イベント・ガチャの取得確認。LINEへの送信はしない"]
    async fn public_push_catalog() {
        let messages = Arc::new(
            Messages::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/messages"),
            )
            .unwrap(),
        );
        let assets = Arc::new(crate::assets::AssetService::new("main").unwrap());
        let source = Source::new(Arc::clone(&assets), Arc::clone(&messages));
        let sale = source.sale().await.unwrap();
        let gacha = source.fetch_lookup_data().await.unwrap();
        let version=assets.get_text("https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/setting/version").await.unwrap().trim().parse().unwrap();
        for (kind, name) in [
            (
                "sale",
                sale.sale_names
                    .values()
                    .max_by_key(|name| name.len())
                    .unwrap(),
            ),
            (
                "g",
                gacha
                    .gacha_names
                    .rare
                    .values()
                    .max_by_key(|name| name.len())
                    .unwrap(),
            ),
        ] {
            let output = crate::push::lookup(
                &source,
                &messages,
                &[kind.into(), "0".into(), "true".into(), name.clone()],
            )
            .await
            .unwrap();
            assert!(output.selection.is_some());
            assert!(output.messages[0].encode_utf16().count() <= 1500);
        }
        let data = Catalog {
            sale: Some(sale),
            gacha: Some(gacha),
            version,
        };
        let now = now_ms();
        let settings = [
            Subscription {
                kind: "all".into(),
                id: 0,
                advance: 0,
                since: now,
            },
            Subscription {
                kind: "g-all".into(),
                id: 0,
                advance: 0,
                since: now,
            },
            Subscription {
                kind: "daily".into(),
                id: 0,
                advance: 0,
                since: now,
            },
        ];
        let notices = collect(&data, &settings, &messages, now, now + 86_400_000).unwrap();
        assert!(notices.iter().any(|n| n.phase == "daily"));
        assert!(
            notices
                .iter()
                .all(|n| crate::commands::split_text(&n.text).len() <= 32
                    && n.contents.as_ref().is_none_or(|parts| parts.len() <= 32))
        );
        eprintln!(
            "Public push catalog: {} notices; event/gacha selection and daily thread valid",
            notices.len()
        );
    }
    use kbc_protocol::{ActionResult, CoreAction, DeliveryStatus};

    #[test]
    fn occurrences_notifications_and_atomic_delivery() {
        let messages = Arc::new(
            Messages::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/messages"),
            )
            .unwrap(),
        );
        let time = |hour: &str| {
            parse_header_date("20261008", hour)
                .unwrap()
                .timestamp_millis()
        };
        let header = serde_json::json!({"startDate":"20261008","startTime":"0000","endDate":"20261010","endTime":"0000","minVersion":"1","maxVersion":"999999"});
        let entry:SaleEntry=serde_json::from_value(serde_json::json!({"header":header,"stageIds":[102,950001],"timeBlocks":[{"dateRanges":[],"monthDays":[],"weekdays":["Thu"],"timeRanges":[["1200","1300"]]}]})).unwrap();
        let map = || ModeMaps {
            rare: std::collections::HashMap::new(),
            event: std::collections::HashMap::new(),
            normal: std::collections::HashMap::new(),
        };
        let mut gacha_names = map();
        gacha_names.rare.insert(7, "テストガチャ".into());
        let data=Catalog {
            sale:Some(SaleDisplayData {sale:SaleJson {_updated_at:String::new(),data:vec![entry.clone()]},sale_names:std::collections::HashMap::from([(950001,"イベント".into()),(102,"全件".into())]),all_day_event_names:Default::default(),mission_names:Default::default(),card_setting_stage_ids:vec![],messages:Arc::clone(&messages)}),
            gacha:Some(GachaLookupData {gacha:serde_json::from_value(serde_json::json!({"data":[{"header":{"startDate":"20261008","startTime":"1200","endDate":"20261008","endTime":"1300","minVersion":"1","maxVersion":"999999","gachaType":1,"gachaCount":1},"gachas":[{"id":7,"price":0,"flags":0,"rates":{"normal":0,"rare":0,"superRare":0,"uberRare":0,"legendRare":0},"guaranteed":false}]}]})).unwrap(),gacha_names,series_names:map(),short_series_names:map(),series_mappings:ModeMaps {rare:Default::default(),event:Default::default(),normal:Default::default()}}),version:1
        };
        let settings = [
            Subscription {
                kind: "sale".into(),
                id: 950001,
                advance: 5,
                since: time("1100"),
            },
            Subscription {
                kind: "all".into(),
                id: 0,
                advance: 0,
                since: time("1100"),
            },
            Subscription {
                kind: "g-all".into(),
                id: 0,
                advance: 0,
                since: time("1100"),
            },
        ];
        let notices = collect(&data, &settings, &messages, time("1149"), time("1300")).unwrap();
        assert_eq!(notices.len(), 3);
        assert_eq!(notices[0].due, time("1155"));
        assert!(
            notices[1].text.contains("950001")
                && notices[1].text.contains("102")
                && notices[1].text.contains("テストガチャ")
        );
        assert!(
            notices[2].text.contains("950001")
                && notices[2].text.contains("テストガチャ")
                && !notices[2].text.contains("102")
        );
        assert!(
            collect(&data, &settings, &messages, time("1300"), time("1301"))
                .unwrap()
                .is_empty()
        );
        let mut overnight = entry.clone();
        overnight.time_blocks[0].time_ranges = vec![["2300".into(), "0100".into()]];
        assert!(
            schedule::periods(
                &overnight,
                DateTime::from_timestamp_millis(time("2200")).unwrap(),
                DateTime::from_timestamp_millis(time("2359")).unwrap()
            )
            .unwrap()
            .iter()
            .any(|(start, end)| end - start == 7_200_000)
        );
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE actions(id TEXT PRIMARY KEY,event_id TEXT,chat TEXT,payload TEXT,due INTEGER,created INTEGER,status TEXT); CREATE TABLE sessions(id TEXT PRIMARY KEY);").unwrap();
        super::super::initialize(&db).unwrap();
        let tx = db.transaction().unwrap();
        let before = collect(&data, &settings, &messages, time("1149"), time("1201"))
            .unwrap()
            .remove(0);
        assert!(!save(&tx, "chat", before, time("1150"), &mut 0).unwrap());
        assert_eq!(
            tx.query_row("SELECT count(*) FROM push_deliveries", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        for _ in 0..2 {
            for notice in collect(&data, &settings, &messages, time("1149"), time("1300")).unwrap()
            {
                assert!(save(&tx, "chat", notice, time("1150"), &mut 64).unwrap());
            }
        }
        assert_eq!(
            tx.query_row("SELECT count(*) FROM actions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let daily_setting = [Subscription {
            kind: "daily".into(),
            id: 0,
            advance: 0,
            since: time("1100"),
        }];
        let notice = collect(&data, &daily_setting, &messages, time("2159"), time("2201"))
            .unwrap()
            .remove(0);
        assert!(notice.contents.is_some());
        assert!(save(&tx, "chat", notice, time("2200"), &mut 64).unwrap());
        tx.execute(
            "INSERT INTO push_subscriptions VALUES('chat','daily',0,0,?1)",
            [time("1100")],
        )
        .unwrap();
        let payload: String = tx
            .query_row(
                "SELECT payload FROM actions WHERE event_id LIKE '%:daily'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let action: CoreAction = serde_json::from_str(&payload).unwrap();
        let CoreAction::SendMessage { ref action_id, .. } = action else {
            panic!("root expected")
        };
        tx.execute(
            "UPDATE actions SET status='sent',payload='' WHERE id=?1",
            [action_id],
        )
        .unwrap();
        crate::skd::complete(
            &tx,
            &action,
            &ActionResult {
                action_id: action_id.clone(),
                status: DeliveryStatus::Sent,
                code: "OK".into(),
                message_id: Some("root".into()),
                oc_result: None,
            },
        )
        .unwrap();
        let due: i64 = tx
            .query_row(
                "SELECT due FROM actions WHERE status='queued' AND json_extract(payload,'$.threadRootId')='root'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(due >= now_ms() + 900);
        super::super::configure(
            &tx,
            "chat",
            &super::super::Choice {
                kind: "sale".into(),
                id: 950002,
                name: String::new(),
            },
            0,
            true,
            &messages,
            now_ms(),
        )
        .unwrap();
        assert_eq!(tx.query_row("SELECT count(*) FROM actions WHERE status='queued' AND json_extract(payload,'$.threadRootId')='root'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        super::super::configure(
            &tx,
            "chat",
            &super::super::Choice {
                kind: "daily".into(),
                id: 0,
                name: String::new(),
            },
            0,
            false,
            &messages,
            now_ms(),
        )
        .unwrap();
        assert_eq!(tx.query_row("SELECT count(*) FROM actions WHERE status='queued' AND json_extract(payload,'$.threadRootId')='root'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        tx.commit().unwrap();
    }
}
