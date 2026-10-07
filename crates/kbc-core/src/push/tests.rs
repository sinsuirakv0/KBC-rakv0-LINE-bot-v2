use super::*;
use crate::{
    Runtime,
    commands::{CommandPlan, sessions},
    now_ms,
};
use kbc_protocol::{ActionResult, DeliveryStatus};

fn open(path: &std::path::Path) -> Runtime {
    Runtime::open(serde_json::from_value(serde_json::json!({"databasePath":path.join("core.sqlite"),"ownerId":"push-test","contentDirectory":std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")})).unwrap()).unwrap()
}
fn event(id: &str, owner: &str, text: &str, reply: Option<&str>) -> CoreEvent {
    serde_json::from_value(serde_json::json!({"type":"messageReceived","eventId":id,"chatId":"chat","messageId":id,"senderId":owner,"senderName":"😀登録者","text":text,"replyToMessageId":reply,"createdAtMs":now_ms()})).unwrap()
}

#[test]
fn reminder_calendar_and_optional_body() {
    let now = crate::skd::model::parse_header_date("20261008", "1200")
        .unwrap()
        .timestamp_millis();
    for args in [
        vec!["5"],
        vec!["10/9-12:00"],
        vec!["2026/10/9", "12:00"],
        vec!["26/10/9(12:00)"],
    ] {
        let (due, body) = time::parse(
            &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            now,
        )
        .unwrap();
        assert!(due > now);
        assert!(body.is_empty());
    }
    assert!(time::parse(&["2/30".into()], now).is_err());
    assert!(time::parse(&["10/9(12:00".into()], now).is_err());
    assert!(time::parse(&["10/9-12:00)".into()], now).is_err());
    assert_eq!(time::parse(&["10/7".into()], now), Err("push.past"));
    assert_eq!(
        time::parse(&["999999999999999".into()], now),
        Err("push.too_far")
    );
    let (due, body) = time::parse(&["5".into(), "本文".into()], now).unwrap();
    assert_eq!(due, now + 300_000);
    assert_eq!(body, "本文");
}

#[tokio::test]
async fn durable_reminder_and_notification_selection() {
    let path =
        std::env::temp_dir().join(format!("kbc-push-test-{}-{}", std::process::id(), now_ms()));
    std::fs::create_dir_all(&path).unwrap();
    let runtime = open(&path);
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        let now = now_ms();
        apply(
            &tx,
            &event("reserve", "alice", "!push 1", None),
            &["1".into()],
            &runtime.content.messages,
            now,
        )
        .unwrap();
        let payload: String = tx
            .query_row(
                "SELECT payload FROM actions WHERE id LIKE 'push:reminder:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            matches!(serde_json::from_str::<CoreAction>(&payload).unwrap(),CoreAction::SendMessage {mention:Some(_),related_message_id,text,..} if related_message_id.is_empty() && text.contains("指定した時間"))
        );
        let choices = (0..12)
            .map(|id| Choice {
                kind: "sale".into(),
                id: 950000 + id,
                name: format!("event{id}"),
            })
            .collect();
        let selection = crate::event_data::Selection {
            kind: "push".into(),
            choices: vec![],
            push: Some(Selection {
                choices,
                page: 0,
                advance: 5,
                enabled: true,
            }),
        };
        tx.execute("INSERT INTO sessions(id,chat,owner,prompt,action,payload,expires,revision) VALUES('select','chat','alice','prompt','old',?1,?2,'push-v1')",params![serde_json::to_string(&selection).unwrap(),now+600_000]).unwrap();
        tx.execute(
            "INSERT INTO events VALUES('reserve','',?1)",
            [now - crate::RETENTION_MS - 1],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    for (id, text) in [("retention", "通常会話"), ("reserve", "!push 1")] {
        let receipt=runtime.submit_batch(serde_json::from_value(serde_json::json!({"protocolVersion":kbc_protocol::PROTOCOL_VERSION,"streamKey":"retention","checkpoint":id,"baselineBeforeMs":null,"events":[event(id,"alice",text,None)]})).unwrap()).unwrap();
        if id == "reserve" {
            assert_eq!(receipt.duplicates, 1);
        }
    }
    assert_eq!(
        runtime
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM actions WHERE id LIKE 'push:reminder:%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    runtime.shutdown();
    drop(runtime);
    let runtime = open(&path);
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        assert!(
            sessions::apply(
                &tx,
                &event("wrong", "bob", "次", Some("prompt")),
                CommandPlan::Ignore,
                None,
                &runtime.content.messages,
                now_ms()
            )
            .unwrap()
            .0
            .is_empty()
        );
        let e = event("next", "alice", "次", Some("prompt"));
        let (response, replace) = sessions::apply(
            &tx,
            &e,
            CommandPlan::Ignore,
            None,
            &runtime.content.messages,
            now_ms(),
        )
        .unwrap();
        assert_eq!(replace.as_deref(), Some("prompt"));
        assert!(response[0].0.contains("2/2"));
        runtime
            .enqueue_responses(&tx, "next", "chat", response, replace, now_ms())
            .unwrap();
        let pending = sessions::apply(
            &tx,
            &event("race", "alice", "1", Some("prompt")),
            CommandPlan::Ignore,
            None,
            &runtime.content.messages,
            now_ms(),
        )
        .unwrap()
        .0;
        assert!(!pending.is_empty());
        assert_eq!(
            tx.query_row("SELECT count(*) FROM push_subscriptions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        tx.commit().unwrap();
    }
    let action = runtime.next_action().await.unwrap().unwrap();
    assert!(
        matches!(action,CoreAction::SendMessage {ref action_id,is_prompt:true,..} if action_id=="next:0")
    );
    runtime.mark_sending("next:0").unwrap();
    runtime
        .complete_action(ActionResult {
            action_id: "next:0".into(),
            status: DeliveryStatus::Sent,
            code: "OK".into(),
            message_id: Some("new-prompt".into()),
            oc_result: None,
        })
        .unwrap();
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        assert!(
            sessions::apply(
                &tx,
                &event("old", "alice", "1", Some("prompt")),
                CommandPlan::Ignore,
                None,
                &runtime.content.messages,
                now_ms()
            )
            .unwrap()
            .0
            .is_empty()
        );
        sessions::apply(
            &tx,
            &event("choose", "alice", "1", Some("new-prompt")),
            CommandPlan::Ignore,
            None,
            &runtime.content.messages,
            now_ms(),
        )
        .unwrap();
        assert_eq!(
            tx.query_row("SELECT target FROM push_subscriptions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            950010
        );
        assert_eq!(
            tx.query_row("SELECT advance FROM push_subscriptions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5
        );
        tx.execute(
            "UPDATE actions SET due=?1 WHERE id LIKE 'push:reminder:%'",
            [now_ms()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    // 次の受信なしで期限通知が取り出せ、通信開始後の再起動はunknownのまま残る。
    let action = loop {
        let action = runtime.next_action().await.unwrap().unwrap();
        if let CoreAction::DeleteMessage { action_id, .. } = action {
            runtime
                .complete_action(ActionResult {
                    action_id,
                    status: DeliveryStatus::Failed,
                    code: "TEST".into(),
                    message_id: None,
                    oc_result: None,
                })
                .unwrap();
        } else {
            break action;
        }
    };
    let CoreAction::SendMessage { action_id, .. } = action else {
        panic!("notification expected")
    };
    assert!(action_id.starts_with("push:reminder:"));
    runtime.mark_sending(&action_id).unwrap();
    runtime.shutdown();
    drop(runtime);
    let runtime = open(&path);
    assert_eq!(
        runtime
            .database
            .lock()
            .unwrap()
            .query_row("SELECT status FROM actions WHERE id=?1", [action_id], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unknown"
    );
    {
        let mut db = runtime.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            &event("paused", "alice", "!push 1", None),
            &["1".into()],
            &runtime.content.messages,
            now_ms(),
        )
        .unwrap();
        tx.execute(
            "UPDATE actions SET due=?1 WHERE id='push:reminder:paused:0'",
            [now_ms()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO bot_stops VALUES('chat','admin',?1)",
            [now_ms()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), runtime.next_action())
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT code FROM actions WHERE id='push:reminder:paused:0'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "BotStopped"
    );
    runtime.shutdown();
    drop(runtime);
    // このテスト自身が作った固定prefixの一時フォルダだけを消す。
    assert!(path.starts_with(std::env::temp_dir()));
    std::fs::remove_dir_all(path).unwrap();
}
