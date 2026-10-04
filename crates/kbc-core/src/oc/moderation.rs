use super::*;

fn notice(tx: &Transaction<'_>, key: &str, window: i64, now: i64) -> Result<bool> {
    let at: Option<i64> = tx
        .query_row("SELECT at FROM oc_notices WHERE id=?1", [key], |r| r.get(0))
        .optional()?;
    if at.is_some_and(|at| at > now - window) {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO oc_notices VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET at=excluded.at",
        params![key, now],
    )?;
    tx.execute("DELETE FROM oc_notices WHERE id IN (SELECT id FROM oc_notices ORDER BY at DESC LIMIT -1 OFFSET 1024)",[])?;
    Ok(true)
}
fn delete(tx: &Transaction<'_>, event: &CoreEvent, reason: &str, now: i64) -> Result<()> {
    let (root, chat, message, sender, _) = identity(event);
    let id = format!("{root}:oc:delete");
    let action = CoreAction::DeleteMessage {
        action_id: id.clone(),
        event_id: root.into(),
        chat_id: chat.into(),
        message_id: message.into(),
        created_at_ms: now,
    };
    insert_action(tx, &action, now)?;
    tx.execute("INSERT OR IGNORE INTO oc_history(square,target,actor,operation,status,detail,at,action) VALUES(?1,?2,'system','delete','受付',?3,?4,?5)",params![square(event).unwrap_or(""),sender,reason,now,id])?;
    tx.execute("DELETE FROM oc_history WHERE id IN (SELECT id FROM oc_history ORDER BY id DESC LIMIT -1 OFFSET 2048)",[])?;
    Ok(())
}
pub fn mute(_runtime: &Runtime, tx: &Transaction<'_>, event: &CoreEvent, now: i64) -> Result<bool> {
    let Some(square) = square(event) else {
        return Ok(false);
    };
    let (root, chat, _, sender, at) = identity(event);
    let value = settings(tx, square)?;
    if value.bot_member.as_deref() == Some(sender) {
        return Ok(false);
    }
    let Some(mute) = value
        .mutes
        .get(sender)
        .filter(|mute| mute.until.is_none_or(|until| until > now) && at >= mute.since)
    else {
        return Ok(false);
    };
    delete(tx, event, "mute", now)?;
    if notice(tx, &format!("mute:{square}:{sender}"), 60000, now)? {
        text_action(
            tx,
            event,
            "mute-notice",
            chat,
            format!(
                "@{}\n現在ミュートされています。期限まで待つか、手動解除をお待ちください。\n残り時間: {}",
                mute.name,
                mute.until
                    .map_or("無期限".into(), |until| policy::duration(until - now))
            ),
            TextDelivery {
                mention: Some(MessageMention {
                    member_id: sender.into(),
                    start: 0,
                    end: format!("@{}", mute.name).encode_utf16().count() as u32,
                }),
                ..Default::default()
            },
            now,
        )?;
    }
    let _ = root;
    Ok(true)
}
pub fn candidate(
    tx: &Transaction<'_>,
    event: &CoreEvent,
    plan: &crate::commands::CommandPlan,
    now: i64,
) -> Result<Option<String>> {
    let Some(square) = square(event) else {
        return Ok(None);
    };
    let (_, chat, message, sender, at) = identity(event);
    let CoreEvent::MessageReceived {
        text,
        content_type,
        media_group_sequence,
        media_group_total,
        ..
    } = event
    else {
        return Ok(None);
    };
    if !sender.starts_with('p') {
        return Ok(None);
    }
    let settings = settings(tx, square)?;
    if !(settings.url || settings.media || settings.danger || settings.cohort) {
        return Ok(None);
    }
    let url_text = policy::url_text(text, !matches!(plan, crate::commands::CommandPlan::Ignore));
    tx.execute("UPDATE oc_members SET messages=messages+1,last_text=?3 WHERE square=?1 AND member=?2 AND state='JOINED'",params![square,sender,text.chars().take(160).collect::<String>()])?;
    if settings.url {
        let urls = policy::urls(url_text);
        if urls.len() >= 65
            || urls
                .iter()
                .any(|url| !settings.rules.iter().any(|rule| rule.matches(url)))
        {
            return Ok(Some("url".into()));
        }
    }
    if settings.danger && policy::danger_word(text).is_some() {
        let first:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM oc_members WHERE square=?1 AND member=?2 AND first=1 AND visits=1 AND joined BETWEEN ?3 AND ?4)",params![square,sender,at-120000,at],|r|r.get(0))?;
        if first {
            return Ok(Some("danger".into()));
        }
    }
    if settings.cohort
        && (policy::danger_word(text).is_some()
            || !policy::urls(url_text).is_empty()
            || ["openchat", "オプチャ", "招待", "宣伝"]
                .iter()
                .any(|word| text.contains(word)))
    {
        let watched:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM oc_members WHERE square=?1 AND member=?2 AND cohort>?3 AND suspicious=0)",params![square,sender,now],|r|r.get(0))?;
        if watched {
            return Ok(Some("cohort".into()));
        }
    }
    if settings.media
        && content_type
            .as_deref()
            .is_some_and(|kind| matches!(kind, "1" | "2" | "21" | "IMAGE" | "VIDEO" | "EXTIMAGE"))
    {
        tx.execute("DELETE FROM oc_media WHERE at<?1", [now - 30000])?;
        tx.execute(
            "INSERT OR IGNORE INTO oc_media VALUES(?1,?2,?3,?4,?5)",
            params![square, sender, message, chat, at],
        )?;
        tx.execute("DELETE FROM oc_media WHERE rowid IN (SELECT rowid FROM oc_media ORDER BY at DESC,rowid DESC LIMIT -1 OFFSET 4096)",[])?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM oc_media WHERE square=?1 AND member=?2 AND at BETWEEN ?3 AND ?4",
            params![square, sender, at - 30000, at],
            |r| r.get(0),
        )?;
        if media_group_total.is_some_and(|total| total >= 7) && media_group_sequence.is_some() {
            if media_group_sequence.is_some_and(|sequence| sequence >= 7) {
                return Ok(Some("media".into()));
            }
        } else if count >= 7 {
            return Ok(Some("media".into()));
        }
    }
    Ok(None)
}
pub fn execute(runtime: &Runtime, tx: &Transaction<'_>, job: &mut Job, now: i64) -> Result<()> {
    let context = job.context.as_ref().ok_or("MissingOcContext")?.clone();
    if policy::role_rank(&context.actor.role) == 0 {
        return Ok(());
    }
    if bot_rank(runtime, job) > 0
        || policy::role_rank(&context.actor.role) >= 2
        || context.actor.member_id == context.bot_member_id
    {
        if let Some(plan) = &job.deferred {
            runtime.apply_command(tx, &job.event, serde_json::from_str(plan)?, now)?;
        }
        return Ok(());
    }
    let value = settings(tx, &context.square_id)?;
    let reason = job.input.args.first().map(String::as_str).unwrap_or("");
    let chat = identity(&job.event).1.to_owned();
    match reason {
        "url" if value.url => {
            let CoreEvent::MessageReceived { text, .. } = &job.event else {
                return Ok(());
            };
            let plan = job
                .deferred
                .as_ref()
                .map(|value| serde_json::from_str::<crate::commands::CommandPlan>(value))
                .transpose()?;
            let urls = policy::urls(policy::url_text(
                text,
                plan.as_ref()
                    .is_some_and(|value| !matches!(value, crate::commands::CommandPlan::Ignore)),
            ));
            let blocked = urls.len() >= 65
                || urls
                    .iter()
                    .any(|url| !value.rules.iter().any(|rule| rule.matches(url)));
            let denied = urls
                .iter()
                .filter(|url| !value.rules.iter().any(|rule| rule.matches(url)))
                .filter(|url| url.as_str().len() <= 1024)
                .take(3)
                .map(|url| url.to_string())
                .collect::<Vec<_>>();
            if !blocked {
                if let Some(plan) = plan {
                    runtime.apply_command(tx, &job.event, plan, now)?;
                }
                return Ok(());
            }
            delete(tx, &job.event, "未許可URL", now)?;
            for url in denied {
                case(
                    tx,
                    job,
                    None,
                    Some(&url),
                    "未許可URL。1 完全一致 / 2 パス / 3 配下 / 4 ドメイン / 5 却下",
                    now,
                )?;
            }
            reply(
                tx,
                job,
                "未許可URLを含む投稿の削除を要求しました。HTTPSの許可ルールは管理者が設定できます。",
                now,
            )
        }
        "danger" if value.danger => {
            job.operation = "danger-kick".into();
            job.targets = vec![context.actor.member_id.clone()];
            if value.report {
                let message = identity(&job.event).2.to_owned();
                return request(
                    tx,
                    job,
                    OcRequest::Report {
                        square_id: context.square_id,
                        message_id: message,
                    },
                    Phase::Report,
                    now,
                );
            }
            delete(tx, &job.event, "初参加・危険語", now)?;
            request(
                tx,
                job,
                OcRequest::Membership {
                    square_id: context.square_id,
                    member_id: context.actor.member_id,
                    revision: context.actor.revision,
                    state: "KICK_OUT".into(),
                },
                Phase::Mutation,
                now,
            )
        }
        "media" if value.media => {
            delete(tx, &job.event, "画像・動画連投", now)?;
            if notice(
                tx,
                &format!("media:{}:{}", context.square_id, context.actor.member_id),
                30000,
                now,
            )? {
                text_action(tx,&job.event,"media-notice",&chat,"7枚以上画像（動画）を連投したため、一部画像の削除を要求しました。\nラグ軽減のため、7枚以上はスレッドへ送信してください。".into(),TextDelivery::default(), now)?;
            }
            Ok(())
        }
        "cohort" if value.cohort => {
            tx.execute(
                "UPDATE oc_members SET suspicious=1 WHERE square=?1 AND member=?2",
                params![context.square_id, context.actor.member_id],
            )?;
            case(
                tx,
                job,
                Some(&context.actor.member_id),
                None,
                "一斉参加の監視対象者による危険語・URL・招待らしい投稿（自動処分なし）。",
                now,
            )
        }
        _ => {
            if let Some(plan) = &job.deferred {
                runtime.apply_command(tx, &job.event, serde_json::from_str(plan)?, now)?;
            }
            Ok(())
        }
    }
}
pub fn after_report(
    tx: &Transaction<'_>,
    job: &mut Job,
    result: &ActionResult,
    now: i64,
) -> Result<()> {
    job.results
        .push(format!("原因投稿の通報: {:?}", result.status));
    delete(tx, &job.event, "初参加・危険語", now)?;
    let context = job.context.as_ref().ok_or("MissingOcContext")?.clone();
    request(
        tx,
        job,
        OcRequest::Membership {
            square_id: context.square_id,
            member_id: context.actor.member_id,
            revision: context.actor.revision,
            state: "KICK_OUT".into(),
        },
        Phase::Mutation,
        now,
    )
}
pub fn case(
    tx: &Transaction<'_>,
    job: &Job,
    target: Option<&str>,
    url: Option<&str>,
    reason: &str,
    now: i64,
) -> Result<()> {
    let context = job.context.as_ref().ok_or("MissingOcContext")?;
    let Some(room) = settings(tx, &context.square_id)?.mod_room else {
        return Ok(());
    };
    tx.execute("DELETE FROM oc_cases WHERE expires<=?1", [now])?;
    if tx.query_row("SELECT count(*) FROM oc_cases", [], |r| r.get::<_, i64>(0))? >= 256 {
        return Ok(());
    }
    let id = format!(
        "{}:case:{}",
        identity(&job.event).0,
        url.unwrap_or(target.unwrap_or(reason))
    );
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM oc_cases WHERE id=?1)",
        [&id],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let suffix = format!(
        "case-{}-{}",
        job.step,
        tx.query_row("SELECT count(*) FROM oc_cases", [], |r| r.get::<_, i64>(0))?
    );
    let target_name = job
        .target_member
        .as_ref()
        .filter(|member| Some(member.member_id.as_str()) == target)
        .or_else(|| (Some(context.actor.member_id.as_str()) == target).then_some(&context.actor))
        .map_or("メンバー", |member| member.name.as_str());
    let text = format!(
        "OC管理ログ\n{reason}\n対象: {}\n実行トーク: {}\n{}\n投稿・参加時刻: {}",
        target.map_or("URL".into(), |target| format!("{target_name} ({target})")),
        identity(&job.event).1,
        url.unwrap_or(""),
        identity(&job.event).4
    );
    let action = text_action(
        tx,
        &job.event,
        &suffix,
        &room,
        text,
        TextDelivery {
            related_message_id: (job.input.name == "moderate"
                && job
                    .input
                    .args
                    .first()
                    .is_some_and(|reason| reason == "cohort"))
            .then(|| identity(&job.event).2.to_owned()),
            ..Default::default()
        },
        now,
    )?;
    tx.execute(
        "INSERT INTO oc_cases VALUES(?1,?2,?3,NULL,?4,?5,?6,?7,'open',?8)",
        params![
            id,
            context.square_id,
            room,
            action,
            target.unwrap_or(""),
            url,
            reason,
            now + 7 * 86400000
        ],
    )?;
    Ok(())
}
pub fn member_event(
    _runtime: &Runtime,
    tx: &Transaction<'_>,
    event: &CoreEvent,
    now: i64,
) -> Result<()> {
    let CoreEvent::MemberChanged {
        square_id,
        chat_id,
        member_id,
        display_name,
        scope,
        state,
        member_created_at_ms,
        created_at_ms,
        ..
    } = event
    else {
        return Ok(());
    };
    if !matches!(scope.as_str(), "square" | "chat")
        || !matches!(state.as_str(), "JOINED" | "LEFT" | "KICK_OUT" | "BANNED")
        || !member_id.starts_with('p')
    {
        return Ok(());
    }
    let value = settings(tx, square_id)?;
    if value.bot_member.as_ref() == Some(member_id) {
        return Ok(());
    }
    let presence_chat = if scope == "square" {
        square_id.as_str()
    } else {
        chat_id.as_str()
    };
    let previous: Option<(String, i64, String)> = tx
        .query_row(
            "SELECT state,at,name FROM oc_presence WHERE square=?1 AND chat=?2 AND member=?3",
            params![square_id, presence_chat, member_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if previous
        .as_ref()
        .is_some_and(|(old, at, _)| old == state || at > created_at_ms)
    {
        return Ok(());
    }
    let name = if display_name.is_empty() {
        previous
            .as_ref()
            .map(|(_, _, name)| name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or("メンバー".into())
    } else {
        display_name.clone()
    };
    tx.execute("INSERT INTO oc_presence VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(square,chat,member) DO UPDATE SET state=excluded.state,at=excluded.at,name=excluded.name",params![square_id,presence_chat,member_id,state,created_at_ms,name])?;
    tx.execute("DELETE FROM oc_presence WHERE rowid IN (SELECT rowid FROM oc_presence ORDER BY at DESC,rowid DESC LIMIT -1 OFFSET 8192)",[])?;
    let destination = if scope == "square" {
        value.main.as_deref()
    } else {
        Some(chat_id.as_str())
    };
    if let Some(destination) = destination {
        let notify = notifications(tx, destination)?;
        let template = match state.as_str() {
            "JOINED" => notify.join,
            "LEFT" => notify.leave,
            _ => None,
        };
        if let Some(template) = template
            && notice(
                tx,
                &format!("notify:{destination}:{member_id}:{state}"),
                90000,
                now,
            )?
        {
            let mut text = template.text.replace("<name>", &name);
            let mut mention = None;
            if template.mention {
                let prefix = format!("@{name}");
                mention = Some(MessageMention {
                    member_id: member_id.clone(),
                    start: 0,
                    end: prefix.encode_utf16().count() as u32,
                });
                text = format!("{prefix}\n{text}");
            }
            if template.show_id {
                use base64::Engine;
                use sha1::Digest;
                let digest = sha1::Sha1::digest(format!("{square_id}:{member_id}").as_bytes());
                let id = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(digest)
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(6)
                    .collect::<String>()
                    .to_ascii_lowercase();
                if template.mention {
                    text = format!(
                        "@{name}\nID: {id}\n{}",
                        template.text.replace("<name>", &name)
                    );
                } else {
                    text = format!("ID: {id}\n{text}");
                }
            }
            text_action(
                tx,
                event,
                "member-notify",
                destination,
                text,
                TextDelivery {
                    mention,
                    ..Default::default()
                },
                now,
            )?;
        }
    }
    if !(value.left || value.danger || value.cohort) {
        return Ok(());
    }
    if scope != "square" && value.main.as_ref() != Some(chat_id) {
        return Ok(());
    }
    let old: Option<(i64, bool, i64, String)> = tx
        .query_row(
            "SELECT joined,first,visits,state FROM oc_members WHERE square=?1 AND member=?2",
            params![square_id, member_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if state == "JOINED" {
        if old
            .as_ref()
            .is_some_and(|(_, _, _, state)| state == "JOINED")
        {
            return Ok(());
        }
        let first = old.is_none()
            && member_created_at_ms.is_some_and(|at| {
                at.is_finite() && ((*created_at_ms as f64) - at).abs() <= 120000.0
            });
        let visits = old.as_ref().map_or(1, |(_, _, visits, _)| visits + 1);
        tx.execute("INSERT INTO oc_members(square,member,name,joined,first,visits,state,last) VALUES(?1,?2,?3,?4,?5,?6,'JOINED',?4) ON CONFLICT(square,member) DO UPDATE SET name=excluded.name,joined=excluded.joined,first=excluded.first,visits=excluded.visits,state='JOINED',last=excluded.last,messages=0,last_text='',cohort=0,suspicious=0",params![square_id,member_id,name,created_at_ms,first,visits])?;
        tx.execute("DELETE FROM oc_members WHERE rowid IN (SELECT rowid FROM oc_members ORDER BY last DESC,rowid DESC LIMIT -1 OFFSET 8192)",[])?;
        if value.cohort {
            let count:i64=tx.query_row("SELECT count(*) FROM oc_members WHERE square=?1 AND first=1 AND visits=1 AND joined>?2 AND state='JOINED'",params![square_id,created_at_ms-120000],|r|r.get(0))?;
            if count >= 3 {
                tx.execute("UPDATE oc_members SET cohort=?3 WHERE square=?1 AND first=1 AND visits=1 AND joined>?2 AND state='JOINED'",params![square_id,created_at_ms-120000,now+1800000])?;
                if notice(
                    tx,
                    &format!("cohort:{square_id}:{}", created_at_ms / 120000),
                    120000,
                    now,
                )? {
                    let job = signal_job(event, &value, square_id, &name, "cohort-log");
                    case(
                        tx,
                        &job,
                        Some(member_id),
                        None,
                        "2分以内に初参加者3人以上。30分の監視を開始（自動処分なし）。",
                        now,
                    )?;
                }
            }
        }
    } else if let Some((joined, _, _, _)) = old
        && *created_at_ms >= joined
        && *created_at_ms - joined <= 1800000
        && value.left
        && state == "LEFT"
    {
        let mut job = signal_job(event, &value, square_id, &name, "left-check");
        if scope == "square" {
            tx.execute(
                "UPDATE oc_members SET state='LEFT',last=?3 WHERE square=?1 AND member=?2",
                params![square_id, member_id, created_at_ms],
            )?;
        }
        job.targets = vec![member_id.clone()];
        request(
            tx,
            &mut job,
            OcRequest::Member {
                member_id: member_id.clone(),
            },
            Phase::Target,
            now,
        )?;
    } else if scope == "square" {
        tx.execute(
            "UPDATE oc_members SET state=?3,last=?4 WHERE square=?1 AND member=?2",
            params![square_id, member_id, state, created_at_ms],
        )?;
    }
    Ok(())
}
fn signal_job(
    event: &CoreEvent,
    value: &Settings,
    square: &str,
    name: &str,
    operation: &str,
) -> Job {
    let mut event = event.clone();
    if let CoreEvent::MemberChanged { chat_id, .. } = &mut event {
        *chat_id = value
            .main
            .as_ref()
            .or(value.source_chat.as_ref())
            .or(value.mod_room.as_ref())
            .cloned()
            .unwrap_or(chat_id.clone());
    }
    let actor = OcMember {
        member_id: "system".into(),
        square_id: square.into(),
        name: name.into(),
        role: "system".into(),
        state: "JOINED".into(),
        revision: "0".into(),
    };
    Job {
        event,
        input: Input {
            name: "signal".into(),
            args: vec![],
            body: String::new(),
        },
        phase: Phase::Target,
        step: 0,
        context: Some(OcContext {
            square_id: square.into(),
            chat_name: String::new(),
            bot_member_id: value.bot_member.clone().unwrap_or_default(),
            bot_role: String::new(),
            actor,
            authority: String::new(),
        }),
        targets: vec![],
        results: vec![],
        operation: operation.into(),
        case_id: None,
        deferred: None,
        id_lookup: None,
        target_member: None,
        test: None,
    }
}
pub fn confirm_left(
    tx: &Transaction<'_>,
    job: &mut Job,
    member: &OcMember,
    now: i64,
) -> Result<()> {
    if !matches!(member.state.as_str(), "LEFT" | "4") {
        return Ok(());
    }
    let value = settings(tx, &member.square_id)?;
    if !value.left {
        return Ok(());
    }
    let row:Option<(i64,bool,i64,i64,String)>=tx.query_row("SELECT joined,first,visits,messages,last_text FROM oc_members WHERE square=?1 AND member=?2",params![member.square_id,member.member_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((joined, first, visits, messages, last_text)) = row else {
        return Ok(());
    };
    let elapsed = identity(&job.event).4 - joined;
    if !(0..=1800000).contains(&elapsed)
        || !notice(
            tx,
            &format!("leave:{}:{}:{joined}", member.square_id, member.member_id),
            1800000,
            now,
        )?
    {
        return Ok(());
    }
    tx.execute(
        "UPDATE oc_members SET state='LEFT',last=?3 WHERE square=?1 AND member=?2",
        params![member.square_id, member.member_id, identity(&job.event).4],
    )?;
    let reason = format!(
        "OC全体の退会を確認。\n参加時間: {} / 発言数: {}件\n最後の発言: {}",
        policy::duration(elapsed),
        messages,
        last_text
    );
    if first && visits == 1 && elapsed <= 300000 {
        job.operation = "left-ban".into();
        job.results.push(reason);
        request(
            tx,
            job,
            OcRequest::Membership {
                square_id: member.square_id.clone(),
                member_id: member.member_id.clone(),
                revision: member.revision.clone(),
                state: "BANNED".into(),
            },
            Phase::Mutation,
            now,
        )
    } else if !first || visits != 1 {
        if let Some(room) = value.mod_room {
            text_action(
                tx,
                &job.event,
                "returning-leave-log",
                &room,
                format!(
                    "【監視ログ】再参加者の本OC短時間退室\n対象: {} ({})\n{reason}\n処分: 未実行\n初参加ではないため、自動再参加禁止や審議は行いません。",
                    member.name, member.member_id
                ),
                TextDelivery::default(),
                now,
            )?;
        }
        Ok(())
    } else {
        case(
            tx,
            job,
            Some(&member.member_id),
            None,
            &format!(
                "【確認待ち】参加後30分以内の退会\n{reason}\n処分: 未実行\n再参加禁止 / 無視 / 解除で審議できます。"
            ),
            now,
        )
    }
}
