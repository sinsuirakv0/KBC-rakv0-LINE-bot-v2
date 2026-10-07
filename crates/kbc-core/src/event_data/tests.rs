use super::*;
use crate::commands::{CommandPlan, sessions};
use kbc_protocol::{ActionResult, CoreEvent, DeliveryStatus};

fn open(path: &std::path::Path) -> Runtime {
    Runtime::open(serde_json::from_value(serde_json::json!({
        "databasePath": path.join("core.sqlite"), "ownerId": "event-test",
        "contentDirectory": std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")
    })).unwrap()).unwrap()
}

// 移植時の公開データ確認だけに使い、通常のテストでネットワークへ接続しない。
#[tokio::test]
#[ignore = "公開event/assetsを取得する単発確認"]
async fn public_event_commands() {
    let messages = Arc::new(
        Messages::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/messages"),
        )
        .unwrap(),
    );
    let source = Source::new(
        Arc::new(crate::assets::AssetService::new("main").unwrap()),
        Arc::clone(&messages),
    );
    let gacha = source.fetch_gacha_json().await.unwrap();
    let gacha_id = gacha
        .data
        .iter()
        .flat_map(|block| &block.gachas)
        .find(|entry| entry.id >= 0)
        .unwrap()
        .id
        .to_string();
    let sale = source.fetch_sale_json().await.unwrap();
    let sale_id = sale
        .data
        .iter()
        .flat_map(|entry| &entry.stage_ids)
        .next()
        .unwrap()
        .to_string();
    let item = source.fetch_item_json().await.unwrap();
    let gift_id = item.data[0].gift.gift_type.to_string();
    let now = chrono::DateTime::from_timestamp_millis(now_ms()).unwrap();
    for (command, id) in [("gatya", gacha_id), ("sale", sale_id), ("item", gift_id)] {
        for arguments in [
            vec![],
            vec![id.clone()],
            vec![id.clone(), "j".into()],
            vec![id.clone(), "r".into()],
        ] {
            let texts = match command {
                "gatya" => gatya::run(&source, &messages, &arguments, now)
                    .await
                    .unwrap(),
                "sale" => {
                    sale::run(&source, &messages, &arguments, now, true)
                        .await
                        .unwrap()
                        .messages
                }
                _ => item::run(&source, &messages, &arguments, now)
                    .await
                    .unwrap(),
            };
            assert!(!texts.is_empty() && texts.len() <= 32);
            assert!(texts.iter().all(|text| !text.is_empty()
                && text.encode_utf16().count() <= 1500
                && !text.contains("```")));
            eprintln!("{command} {arguments:?}: {} messages", texts.len());
        }
    }
    let lookup = source.fetch_lookup_data().await.unwrap();
    let (series, name) = lookup
        .gacha
        .data
        .iter()
        .filter(|block| block.header.gacha_type == 1)
        .flat_map(|block| &block.gachas)
        .find_map(|entry| {
            let series = lookup.series_mappings.rare.get(&entry.id)?;
            Some((series, lookup.short_series_names.rare.get(series)?))
        })
        .unwrap();
    for arguments in [
        vec!["R".into(), format!("s{series}")],
        vec!["R".into(), name.clone()],
        vec!["R".into(), format!("s{series}"), "j".into()],
    ] {
        let texts = gatya::run(&source, &messages, &arguments, now)
            .await
            .unwrap();
        assert!(!texts.is_empty() && texts.iter().all(|text| !text.starts_with('❌')));
        eprintln!("gatya series/search: {} messages", texts.len());
    }
    let data = source.sale().await.unwrap();
    let query = data
        .sale_names
        .iter()
        .filter(|(id, _)| !crate::skd::labels::is_mission_id(**id))
        .map(|(_, name)| name)
        .find(|name| {
            data.sale_names
                .values()
                .filter(|candidate| candidate.to_lowercase().contains(&name.to_lowercase()))
                .count()
                == 1
        })
        .unwrap();
    let output = sale::run(&source, &messages, std::slice::from_ref(query), now, true)
        .await
        .unwrap();
    assert!(output.selection.is_some());
    eprintln!("sale name selection: {} messages", output.messages.len());
}

#[tokio::test]
async fn reply_selection_survives_restart_and_checks_owner_and_expiry() {
    let path = std::env::temp_dir().join(format!("kbc-event-{}-{}", std::process::id(), now_ms()));
    std::fs::create_dir_all(&path).unwrap();
    let runtime = open(&path);
    let action = CoreAction::PrepareMedia {
        action_id: "event:0".into(),
        event_id: "event".into(),
        chat_id: "chat".into(),
        related_message_id: String::new(),
        request: String::new(),
        replace_message_id: None,
        is_prompt: false,
        created_at_ms: now_ms(),
    };
    runtime.database.lock().unwrap().execute("INSERT INTO actions(id,event_id,chat,payload,due,created,status) VALUES('event:0','event','chat',?1,0,?2,'preparing')",params![serde_json::to_string(&action).unwrap(),now_ms()]).unwrap();
    runtime
        .finish_event_data(
            action,
            Request {
                command: "sale".into(),
                arguments: vec!["event".into()],
                owner: Some("alice".into()),
            },
            Output {
                messages: vec!["1. 17000 event".into()],
                selection: Some(Selection {
                    kind: "sale".into(),
                    choices: vec![17000],
                }),
            },
        )
        .unwrap();
    assert!(matches!(
        runtime.next_action().await.unwrap(),
        Some(CoreAction::SendMessage {
            is_prompt: true,
            ..
        })
    ));
    runtime.mark_sending("event:0").unwrap();
    runtime
        .complete_action(ActionResult {
            action_id: "event:0".into(),
            status: DeliveryStatus::Sent,
            code: "OK".into(),
            message_id: Some("prompt".into()),
            oc_result: None,
        })
        .unwrap();
    let expires: i64 = runtime
        .database
        .lock()
        .unwrap()
        .query_row("SELECT expires FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!((0..=30_000).contains(&(expires - now_ms())));
    runtime.shutdown();
    drop(runtime);
    let runtime = open(&path);
    let mut db = runtime.database.lock().unwrap();
    let tx = db.transaction().unwrap();
    let event = |owner: &str, input: &str| {
        serde_json::from_value::<CoreEvent>(serde_json::json!({
            "type":"messageReceived","eventId":"reply","chatId":"chat","messageId":"reply",
            "text":input,"senderId":owner,"replyToMessageId":"prompt","createdAtMs":now_ms()
        }))
        .unwrap()
    };
    assert!(
        sessions::apply(
            &tx,
            &event("bob", "1"),
            CommandPlan::Ignore,
            None,
            &runtime.content.messages,
            now_ms()
        )
        .unwrap()
        .0
        .is_empty()
    );
    assert!(
        sessions::apply(
            &tx,
            &event("alice", "1"),
            CommandPlan::Ignore,
            None,
            &runtime.content.messages,
            expires + 1
        )
        .unwrap()
        .0
        .is_empty()
    );
    let (responses, _) = sessions::apply(
        &tx,
        &event("alice", "1"),
        CommandPlan::Ignore,
        None,
        &runtime.content.messages,
        now_ms(),
    )
    .unwrap();
    assert!(
        matches!(&responses[0].2,Some(crate::media::MediaJob {request: crate::media::MediaRequest::EventData(Request {arguments,..}),..}) if arguments == &["17000"])
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    tx.commit().unwrap();
    drop(db);
    runtime.shutdown();
    drop(runtime);
    // 一時ディレクトリはこのテスト自身が作った固定prefixの配下だけを削除する。
    std::fs::remove_dir_all(path).unwrap();
}
