// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::event_reminders::{notice, plan};
use mailrs_domain::calendar::Calendar;
use mailrs_store::{calendar, event_reminders as log};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Default)]
pub struct Reminders {
    // Wait for the frontend's saved preference before evaluating reminders.
    enabled: AtomicU8,
    pending: tokio::sync::Mutex<HashMap<String, plan::Due>>,
}
impl Engine {
    pub async fn new_mail(&self, id: i64, ids: Vec<String>) -> Result<()> {
        let messages = self.db.read(move |c| messages::by_ids(c, id, &ids)).await?;
        let messages: Vec<_> = messages
            .into_iter()
            .filter(|m| m.is_unread() && m.in_role(mailrs_domain::Role::Inbox))
            .collect();
        if messages.is_empty() {
            return Ok(());
        }
        // Preserve the existing private count-only previews. A single message
        // still retains its exact account/thread target for the default action.
        for n in crate::mail_notice::notices(messages, false, vec![]) {
            let key = format!("mail/{id}/{}", now_millis());
            emit(
                &self.out,
                json!({"event":"notification", "kind":"mail", "key":key,
                "title":n.title,"body":n.body,"target":n.target.map(|t|json!({"account":t.account_id,"thread":t.thread_id,"message":t.message_id})),"account":id,
                "expires_at":now_millis()+10*60_000,"actions":[["default",gettext("Open")]]}),
            );
        }
        emit(&self.out, json!({"event":"changed"}));
        Ok(())
    }
    pub async fn check_reminders(&self) -> Result<()> {
        let enabled = self.reminders.enabled.load(Ordering::Relaxed);
        if enabled == 0 {
            return Ok(());
        }
        let now = now_millis();
        // The gate also serializes delivery acknowledgements and snoozes with
        // scheduler reads, preventing a stale plan from reposting a shown item.
        let mut pending = self.reminders.pending.lock().await;
        let (events, calendars, held) = self
            .db
            .read(move |c| {
                let ids: Vec<_> = accounts::list_accounts(c)?
                    .into_iter()
                    .map(|a| a.id)
                    .collect();
                let events = calendar::occurrences(
                    c,
                    &ids,
                    now - plan::LOOK_BACK,
                    now + plan::LOOK_AHEAD,
                    calendar::CalendarScope::Shown,
                )?;
                let mut calendars: HashMap<(i64, String), Calendar> = HashMap::new();
                for id in ids {
                    for cal in calendar::calendars(c, id)? {
                        calendars.insert((id, cal.id.clone()), cal);
                    }
                }
                Ok((events, calendars, log::log(c)?))
            })
            .await?;
        let plan = plan::plan(&events, &calendars, &held, now, enabled == 2);
        let live: std::collections::HashSet<_> = plan
            .post
            .iter()
            .map(|d| plan::notification_id(&d.key))
            .collect();
        for id in pending.keys().filter(|id| !live.contains(*id)) {
            emit(&self.out, json!({"event":"notification-withdraw","key":id}));
        }
        pending.retain(|id, _| live.contains(id));
        let quiet: Vec<_> = plan.quiet.iter().map(|d| (d.key.clone(), d.ends)).collect();
        self.db
            .write(move |c| log::mark_shown(c, &quiet, now))
            .await?;
        for due in plan.post {
            let shown = notice::notice(&due, now, &chrono::Local);
            let mut actions = vec![json!(["default", gettext("Open")])];
            for button in &shown.buttons {
                actions.push(json!([button.action(), button.label()]));
            }
            let date = chrono::DateTime::from_timestamp_millis(due.key.start)
                .map(|at| {
                    if due.occurrence.event.all_day {
                        at.format("%Y-%m-%d").to_string()
                    } else {
                        at.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d")
                            .to_string()
                    }
                })
                .unwrap_or_default();
            pending.insert(shown.id.clone(), due.clone());
            emit(
                &self.out,
                json!({"event":"notification","kind":"calendar","key":shown.id,
                "title":shown.title,"body":shown.body,"target":due.key,"date":date,"expires_at":due.ends,
                "actions":actions}),
            );
        }
        Ok(())
    }
    pub async fn notification_request(&self, r: &Value) -> Result<Value> {
        match string(r, "op") {
            "notification-settings" => {
                self.reminders.enabled.store(
                    if r["reminders"].as_bool().unwrap_or(true) {
                        2
                    } else {
                        1
                    },
                    Ordering::Relaxed,
                );
                self.check_reminders().await?;
            }
            "notification-delivered" => {
                let mut pending = self.reminders.pending.lock().await;
                if let Some(due) = pending.get(string(r, "key")) {
                    let shown = vec![(due.key.clone(), due.ends)];
                    self.db
                        .write(move |c| log::mark_shown(c, &shown, now_millis()))
                        .await?;
                    pending.remove(string(r, "key"));
                }
            }
            "notification-action" => {
                let key: log::Key = serde_json::from_value(r["target"].clone())?;
                // Act on stored calendar data, never a URL supplied by a signal.
                let key2 = key.clone();
                let event = self
                    .db
                    .read(move |c| calendar::event(c, key2.account_id, &key2.calendar, &key2.event))
                    .await?;
                let Some(event) = event else {
                    bail!("{}", gettext("Event not found"));
                };
                match string(r, "action") {
                    "snooze-reminder" => {
                        let mut pending = self.reminders.pending.lock().await;
                        let id = plan::notification_id(&key);
                        let now = now_millis();
                        self.db
                            .write(move |c| log::snooze(c, &key, now, now + plan::SNOOZE))
                            .await?;
                        pending.remove(&id);
                    }
                    "join-event" => {
                        if let Some(link) = event.conference.filter(|v| notice::joinable(v)) {
                            return Ok(json!({"url":link}));
                        }
                    }
                    _ => bail!("{}", gettext("Unknown request")),
                }
            }
            _ => bail!("{}", gettext("Unknown request")),
        }
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailrs_domain::calendar::{Access, Event, Reminder, ReminderMethod};

    #[tokio::test]
    async fn delivery_acknowledgement_snooze_and_disabled_reminders_use_the_store() {
        let engine = Engine::start(true, Arc::new(Mutex::new(io::stdout())))
            .await
            .unwrap();
        let id = engine.accounts().await.unwrap()[0].id;
        let now = now_millis();
        let event = Event {
            calendar: "notification-tests".into(),
            id: "due".into(),
            title: "Due reminder".into(),
            start: now + 60_000,
            end: now + 3_600_000,
            conference: Some("https://example.invalid/meeting".into()),
            reminders: Some(vec![Reminder {
                minutes: 5,
                method: ReminderMethod::Notification,
            }]),
            ..Event::default()
        };
        let key = log::Key {
            account_id: id,
            calendar: event.calendar.clone(),
            event: event.id.clone(),
            start: event.start,
            minutes: 5,
        };
        engine
            .db
            .write(move |c| {
                calendar::save_calendars(
                    c,
                    id,
                    &[Calendar {
                        id: "notification-tests".into(),
                        name: "Test".into(),
                        color: "#123456".into(),
                        access: Access::Owner,
                        zone: "UTC".into(),
                        primary: true,
                        shown: true,
                        hidden: false,
                        reminders: vec![],
                        all_day_reminders: None,
                    }],
                )?;
                calendar::save_events(c, id, &[event], now)
            })
            .await
            .unwrap();
        // No startup race can consume reminders before the preference arrives.
        engine.check_reminders().await.unwrap();
        assert!(engine.reminders.pending.lock().await.is_empty());
        engine
            .notification_request(&json!({"op":"notification-settings","reminders":true}))
            .await
            .unwrap();
        let notice_id = plan::notification_id(&key);
        assert!(
            engine
                .reminders
                .pending
                .lock()
                .await
                .contains_key(&notice_id)
        );
        assert!(
            engine.db.read(log::log).await.unwrap().is_empty(),
            "failed or absent server delivery remains retryable"
        );
        engine.check_reminders().await.unwrap();
        assert_eq!(engine.reminders.pending.lock().await.len(), 1);
        engine
            .notification_request(&json!({"op":"notification-delivered","key":"not-issued"}))
            .await
            .unwrap();
        assert!(engine.db.read(log::log).await.unwrap().is_empty());
        engine
            .notification_request(&json!({"op":"notification-delivered","key":notice_id}))
            .await
            .unwrap();
        assert!(engine.db.read(log::log).await.unwrap().contains_key(&key));
        engine.check_reminders().await.unwrap();
        assert!(
            engine.reminders.pending.lock().await.is_empty(),
            "accepted reminder is not repeated"
        );
        let reply = engine
            .notification_request(
                &json!({"op":"notification-action","action":"join-event","target":key}),
            )
            .await
            .unwrap();
        assert_eq!(reply["url"], "https://example.invalid/meeting");
        engine
            .notification_request(
                &json!({"op":"notification-action","action":"snooze-reminder","target":key}),
            )
            .await
            .unwrap();
        let shown = engine.db.read(log::log).await.unwrap();
        assert!(shown[&key].snoozed_until.unwrap() >= now + plan::SNOOZE);
        engine.check_reminders().await.unwrap();
        assert!(engine.reminders.pending.lock().await.is_empty());
        let overdue = key.clone();
        engine
            .db
            .write(move |c| log::snooze(c, &overdue, now, now - 1))
            .await
            .unwrap();
        engine.check_reminders().await.unwrap();
        assert!(
            engine
                .reminders
                .pending
                .lock()
                .await
                .contains_key(&notice_id),
            "a snoozed reminder becomes due again"
        );
        engine
            .notification_request(&json!({"op":"notification-settings","reminders":false}))
            .await
            .unwrap();
        assert!(engine.reminders.pending.lock().await.is_empty());
        assert_eq!(
            engine.db.read(log::log).await.unwrap()[&key].snoozed_until,
            None
        );
        engine
            .notification_request(&json!({"op":"notification-settings","reminders":true}))
            .await
            .unwrap();
        assert!(
            engine.reminders.pending.lock().await.is_empty(),
            "re-enabling brings no backlog"
        );
        engine.sync.shutdown();
    }
}
