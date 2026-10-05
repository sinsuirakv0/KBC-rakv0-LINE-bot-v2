use super::*;
use chrono::{TimeZone, Utc};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn fixture() -> (PathBuf, Runtime) {
    let path = std::env::temp_dir().join(format!(
        "kbc-skd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let runtime = open(&path);
    (path, runtime)
}
pub(super) fn open(path: &Path) -> Runtime {
    Runtime::open(serde_json::from_value(serde_json::json!({"databasePath":path.join("core.sqlite"),"ownerId":"skd-test","contentDirectory":Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")})).unwrap()).unwrap()
}
fn result(id: &str, status: DeliveryStatus, message: Option<&str>) -> ActionResult {
    ActionResult {
        action_id: id.into(),
        status,
        code: "Test".into(),
        message_id: message.map(String::from),
        oc_result: None,
    }
}
fn insert(runtime: &Runtime, action: &CoreAction) {
    let CoreAction::SendMessage {
        action_id,
        event_id,
        chat_id,
        ..
    } = action
    else {
        unreachable!()
    };
    runtime.database.lock().unwrap().execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES(?1,?2,?3,?4,0,0,'queued')",params![action_id,event_id,chat_id,serde_json::to_string(action).unwrap()]).unwrap();
}

#[tokio::test]
async fn parent_unknown_restart_delay_and_order() {
    let (path, runtime) = fixture();
    let root = root_action(
        &runtime,
        "root:0".into(),
        "root".into(),
        "chat".into(),
        vec!["本文1".into(), "本文2".into()],
        now_ms(),
    )
    .unwrap();
    insert(&runtime, &root);
    assert_eq!(pending_count(&runtime.database.lock().unwrap()).unwrap(), 3);
    let action = runtime.next_action().await.unwrap().unwrap();
    runtime.mark_sending("root:0").unwrap();
    runtime
        .complete_action(result("root:0", DeliveryStatus::Unknown, None))
        .unwrap();
    assert_eq!(
        runtime
            .database
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM actions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(matches!(
        action,
        CoreAction::SendMessage {
            thread_root_id: None,
            ..
        }
    ));
    runtime.shutdown();
    drop(runtime);
    let runtime = open(&path);
    let start = now_ms();
    runtime
        .resolve_action(result("root:0", DeliveryStatus::Sent, Some("root-message")))
        .unwrap();
    let due: i64 = runtime
        .database
        .lock()
        .unwrap()
        .query_row(
            "SELECT min(due) FROM actions WHERE status='queued'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(due >= start + THREAD_READY_DELAY_MS);
    runtime.shutdown();
    drop(runtime);
    let runtime = open(&path);
    let first = runtime.next_action().await.unwrap().unwrap();
    assert!(now_ms() >= due);
    assert!(
        matches!(&first,CoreAction::SendMessage{thread_root_id:Some(root),text,..} if root=="root-message" && text=="本文1")
    );
    runtime.mark_sending("root:0:thread:0").unwrap();
    runtime
        .complete_action(result("root:0:thread:0", DeliveryStatus::Unknown, None))
        .unwrap();
    // 不明な先行本文を飛び越えず、無関係の通常返信は止めない。
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        runtime
            .enqueue_responses(
                &tx,
                "ping",
                "chat",
                vec![("pong".into(), now_ms(), None)],
                None,
                now_ms(),
            )
            .unwrap();
        tx.commit().unwrap();
    }
    let ping = runtime.next_action().await.unwrap().unwrap();
    assert!(matches!(ping,CoreAction::SendMessage{text,..} if text=="pong"));
    runtime.mark_sending("ping:0").unwrap();
    runtime
        .complete_action(result("ping:0", DeliveryStatus::Sent, Some("ping-id")))
        .unwrap();
    runtime
        .resolve_action(result(
            "root:0:thread:0",
            DeliveryStatus::Sent,
            Some("first-id"),
        ))
        .unwrap();
    let second = runtime.next_action().await.unwrap().unwrap();
    assert!(matches!(second,CoreAction::SendMessage{text,..} if text=="本文2"));
    runtime.mark_sending("root:0:thread:1").unwrap();
    runtime
        .complete_action(result(
            "root:0:thread:1",
            DeliveryStatus::Sent,
            Some("second-id"),
        ))
        .unwrap();
    let event = "store:skd:300:chat";
    let id = format!("{event}:0");
    runtime
        .database
        .lock()
        .unwrap()
        .execute("INSERT INTO store_subscriptions VALUES('skd','chat')", [])
        .unwrap();
    insert(
        &runtime,
        &root_action(
            &runtime,
            id.clone(),
            event.into(),
            "chat".into(),
            vec!["本文".into()],
            now_ms(),
        )
        .unwrap(),
    );
    let root = runtime.next_action().await.unwrap().unwrap();
    assert!(matches!(root, CoreAction::SendMessage { .. }));
    runtime.mark_sending(&id).unwrap();
    // 送信中に解除され、親が後から成功した場合も本文は生成しない。
    let input=serde_json::from_value(serde_json::json!({"type":"messageReceived","eventId":"off","chatId":"chat","messageId":"off","text":"!pushsetting skd off","createdAtMs":now_ms()})).unwrap();
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        crate::store_update::configure(&runtime, &tx, &input, &["skd".into(), "off".into()])
            .unwrap();
        tx.commit().unwrap();
    }
    runtime
        .complete_action(result(&id, DeliveryStatus::Sent, Some("off-root-id")))
        .unwrap();
    assert_eq!(
        runtime
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM actions WHERE event_id=?1 AND status='queued'",
                [event],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    runtime.shutdown();
    drop(runtime);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn parses_diff_and_formats_plain_text() {
    let (path, runtime) = fixture();
    assert_eq!(
        parse_date(&["2026", "10", "05"]).unwrap(),
        Some("2026-10-05".into())
    );
    assert!(parse_date(&["2026/02/30"]).is_err());
    let before = "20261001\t1100\t20261010\t1100\t150000\t999999\t0\t0\t2\t100\t101";
    let after = "20261001\t1100\t20261012\t1100\t150000\t999999\t0\t0\t3\t100\t101\t102";
    let (sale, changed) = diff::compare_sale(
        parser::parse_sale_tsv(before).unwrap(),
        parser::parse_sale_tsv(after).unwrap(),
    )
    .unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(sale.data[0].stage_ids, vec![102]);
    let messages = Arc::clone(&runtime.content.messages);
    let data = formatter::AddedScheduleData {
        sale: Some(model::SaleDisplayData {
            sale,
            sale_names: HashMap::from([
                (100, "旧イベントA".into()),
                (101, "旧イベントB".into()),
                (102, "新イベント".into()),
            ]),
            all_day_event_names: HashMap::new(),
            mission_names: HashMap::new(),
            card_setting_stage_ids: Vec::new(),
            messages: Arc::clone(&messages),
        }),
        changes: diff::ScheduleChanges {
            sale: changed,
            ..Default::default()
        },
        ..Default::default()
    };
    let text = formatter::format_added_schedules(
        &data,
        Utc.with_ymd_and_hms(2026, 10, 5, 0, 0, 0).unwrap(),
        "https://example.com",
        &messages,
    )
    .unwrap();
    let all = text.join("\n");
    assert!(
        all.contains("102 新イベント") && all.contains("終了: ") && all.contains("旧イベントA")
    );
    assert!(text.iter().all(|text| !text.contains("```")
        && !text.contains("**")
        && text.encode_utf16().count() <= 1500));
    assert!(validate_contents(&vec!["x".into(); MAX_THREAD_MESSAGES + 1]).is_err());
    drop(runtime);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
#[ignore = "公開データ取得の単発確認。LINEへは送信しない"]
async fn public_schedule_source() {
    let (path, runtime) = fixture();
    let contents = runtime.prepare_schedule(None).await.unwrap();
    assert!(contents.len() >= 2 && contents.len() <= MAX_THREAD_MESSAGES);
    eprintln!("Schedule messages: {}", contents.len());
    let hashes=runtime.assets.get_text("https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/state/skd-notifications.json").await.unwrap();
    let hashes = monitor::parse_hashes(&hashes).unwrap();
    let source = source::SkdDataSource::new(
        Arc::new(runtime.assets.clone()),
        Arc::clone(&runtime.content.messages),
    );
    assert!(source.latest_timestamp(&hashes).await.unwrap() > 0);
    // 最新の更新に含まれない種別の辞書も単発で確認する。
    let metadata = metadata::Metadata::new(
        Arc::new(runtime.assets.clone()),
        Arc::clone(&runtime.content.messages),
    );
    let gatya = metadata
        .gatya(model::GachaJson {
            _updated_at: String::new(),
            data: Vec::new(),
        })
        .await
        .unwrap();
    assert!(
        !gatya.series_mappings.rare.is_empty()
            && !gatya.series_mappings.event.is_empty()
            && !gatya.series_mappings.normal.is_empty()
    );
    let sale = metadata
        .sale(model::SaleJson {
            _updated_at: String::new(),
            data: Vec::new(),
        })
        .await
        .unwrap();
    assert!(
        !sale.sale_names.is_empty()
            && !sale.mission_names.is_empty()
            && !sale.all_day_event_names.is_empty()
    );
    let item = metadata
        .item(model::ItemJson {
            _updated_at: String::new(),
            data: Vec::new(),
        })
        .await
        .unwrap();
    assert!(!item.item_names.is_empty());
    drop(runtime);
    std::fs::remove_dir_all(path).unwrap();
}
