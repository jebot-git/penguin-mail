// SPDX-License-Identifier: GPL-3.0-or-later
//! The FLTK window talks to the upstream engines over private inherited pipes.
//! Mail, passwords and tokens never enter command-line arguments or a socket.
#[path = "accounts.rs"]
mod account_forms;
mod clock_format;
mod compose;
mod crypto;
mod event_reminders;
#[allow(dead_code)]
#[path = "../../../app/src/notify.rs"]
mod mail_notice;
mod notifications;
mod oauth;
// These shared adapters also contain S/MIME wording used by the GTK app.
mod demo;
#[allow(dead_code)]
#[path = "../../../app/src/pgp/adapter.rs"]
mod pgp;
#[allow(dead_code)]
#[path = "../../../app/src/protection/common.rs"]
mod protection;
use anyhow::{Context, Result, bail};
use mailrs_domain::translate::gettext;
use mailrs_domain::{Account, Address, ChangeEvent, Folder, Target};
use mailrs_store::{Db, accounts, labels, messages, threads};
use mailrs_sync::calendar_copy::CalendarCopy;
use mailrs_sync::config::{self, Config};
use mailrs_sync::mailbox::Standard;
use mailrs_sync::passwords::Secrets;
use mailrs_sync::sign_in::{self, Browser, Wanted};
use mailrs_sync::{
    Calendar, Clients, Connected, Connector, ContactBook, History, Loaded, MailAction, MailActions,
    Mailbox, Mailboxes, OneClick, Outbox, Permitted, Scope, SyncEngine, TriageAction, View,
    now_millis,
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

const MAX_REQUEST: usize = 64 * 1024 * 1024;
type Output = Arc<Mutex<io::Stdout>>;
fn emit(out: &Output, value: Value) {
    if let Ok(mut out) = out.lock() {
        let _ = serde_json::to_writer(&mut *out, &value);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}
fn string<'a>(r: &'a Value, key: &str) -> &'a str {
    r[key].as_str().unwrap_or("")
}
fn account(r: &Value) -> i64 {
    r["account"].as_i64().unwrap_or(0)
}
fn permitted<T>(v: Permitted<T>) -> Result<T> {
    match v {
        Permitted::Done(v) => Ok(v),
        Permitted::NeedsPermission => bail!(
            "{}",
            mailrs_domain::translate::gettext("Sign in again to grant permission.")
        ),
    }
}
struct Engine {
    db: Db,
    sync: Arc<SyncEngine>,
    connector: Connector,
    actions: MailActions<SyncEngine>,
    boxes: Mailboxes<SyncEngine>,
    outbox: Outbox<SyncEngine>,
    copy: Arc<CalendarCopy<SyncEngine>>,
    calendar: Calendar<SyncEngine>,
    contacts: ContactBook<SyncEngine>,
    out: Output,
    demo: bool,
    reminders: notifications::Reminders,
    pgp: Option<mailrs_pgp::Pgp>,
    crypto_gate: Arc<tokio::sync::Semaphore>,
    // Mutation order follows the order in which commands arrive.
    writes: Arc<tokio::sync::Mutex<()>>,
    _lock: Option<mailrs_sync::lock::SyncLock>,
    _temporary: Option<tempfile::TempDir>,
}
impl Engine {
    async fn start(is_demo: bool, out: Output) -> Result<Arc<Self>> {
        let temporary = if is_demo {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let dir = match &temporary {
            Some(d) => d.path().to_path_buf(),
            None => config::data_dir()?,
        };
        config::private_dir(&dir)?;
        let lock = mailrs_sync::lock::SyncLock::take(&dir)?;
        let db = Db::open(&dir.join("mailrs.db"))?;
        let config = if is_demo {
            Config::default()
        } else {
            match Config::load(&config::config_path()?) {
                Ok(c) => c,
                Err(e) if e.is_missing() => Config::default(),
                Err(e) => return Err(e.into()),
            }
        };
        let connector = Connector {
            db: db.clone(),
            config: config.clone(),
            clients: Clients::current(),
            secrets: Secrets::keyring(),
        };
        let (sync, events) = SyncEngine::new(db.clone(), config.engine_config());
        let sync = Arc::new(sync);
        if is_demo {
            demo::seed(&db, &sync).await?;
        }
        let copy = Arc::new(CalendarCopy::new(sync.clone(), db.clone()));
        let pgp = mailrs_pgp::Pgp::find().ok().map(|pgp| {
            if is_demo {
                pgp.with_home(dir.join("gnupg"))
            } else {
                pgp
            }
        });
        if is_demo {
            config::private_dir(&dir.join("gnupg"))?;
        }
        let engine = Arc::new(Self {
            actions: MailActions::new(sync.clone(), db.clone(), OneClick::Web),
            boxes: Mailboxes::new(sync.clone(), db.clone()),
            outbox: Outbox::new(sync.clone(), db.clone()),
            calendar: Calendar::new(sync.clone(), db.clone(), copy.clone()),
            contacts: ContactBook::new(sync.clone(), db.clone(), dir.join("contact-photos")),
            db,
            sync,
            connector,
            copy,
            out,
            demo: is_demo,
            reminders: notifications::Reminders::default(),
            pgp,
            crypto_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            writes: Arc::new(tokio::sync::Mutex::new(())),
            _lock: Some(lock),
            _temporary: temporary,
        });
        let e = engine.clone();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    ChangeEvent::NewMail {
                        account_id,
                        message_ids,
                    } => {
                        if let Err(error) = e.new_mail(account_id, message_ids).await {
                            tracing::warn!(%error, "could not prepare mail notification");
                        }
                    }
                    _ => emit(&e.out, json!({"event":"changed"})),
                }
            }
        });
        Ok(engine)
    }
    async fn accounts(&self) -> Result<Vec<Account>> {
        Ok(self.db.read(accounts::list_accounts).await?)
    }
    async fn connect(&self, a: &Account) -> Result<()> {
        match self.connector.connect(a).await? {
            Connected::Ready(s) => {
                self.sync.start_account(a.id, s);
                Ok(())
            }
            Connected::NeedsSignIn(_) => bail!(
                "{}: {}",
                a.email,
                mailrs_domain::translate::gettext("Sign in again")
            ),
        }
    }
    async fn background(self: Arc<Self>) {
        if !self.demo {
            for a in self.accounts().await.unwrap_or_default() {
                if let Err(e) = self.connect(&a).await {
                    emit(
                        &self.out,
                        json!({"event":"account-error", "account":a.id,"error":e.to_string()}),
                    );
                }
            }
        }
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let ids: Vec<_> = self
                .accounts()
                .await
                .unwrap_or_default()
                .iter()
                .map(|a| a.id)
                .collect();
            if let Err(e) = self
                .copy
                .refresh_due(&ids, now_millis(), self.sync.window_open())
                .await
            {
                emit(&self.out, json!({"event":"warning","error":e.to_string()}));
            }
            if let Err(e) = self.outbox.send_due(now_millis()).await {
                emit(&self.out, json!({"event":"warning","error":e.to_string()}));
            }
            for id in ids {
                let _ = self.contacts.refresh_stale(&[id], now_millis()).await;
            }
            emit(&self.out, json!({"event":"changed"}));
        }
    }
    fn mailbox(&self, r: &Value) -> Result<Mailbox> {
        let id = r["account"].as_i64().filter(|id| *id > 0);
        let name = string(r, "mailbox");
        let standard = match name {
            "inbox" => Some(Standard::Inbox),
            "flagged" => Some(Standard::Flagged),
            "sent" => Some(Standard::Sent),
            "drafts" => Some(Standard::Drafts),
            "muted" => Some(Standard::Muted),
            _ => None,
        };
        if let Some(which) = standard {
            return Ok(match id {
                Some(account_id) => Mailbox::Standard { account_id, which },
                None => Mailbox::Unified(which),
            });
        }
        Ok(match name {
            "search" => Mailbox::Search {
                query: string(r, "query").into(),
                account_id: id,
            },
            "archive" => Mailbox::Folder {
                account_id: id,
                folder: Folder::Archive,
            },
            "trash" => Mailbox::Folder {
                account_id: id,
                folder: Folder::Trash,
            },
            "junk" => Mailbox::Folder {
                account_id: id,
                folder: Folder::Junk,
            },
            "all" => Mailbox::Folder {
                account_id: id,
                folder: Folder::AllMail,
            },
            "outbox" => Mailbox::Outbox,
            "scheduled" => Mailbox::Scheduled,
            "reminders" => Mailbox::Reminders,
            "follow-up" => Mailbox::FollowUp,
            "label" => Mailbox::Label {
                account_id: id.context(gettext("Account required"))?,
                label_id: string(r, "label").into(),
                name: string(r, "name").into(),
            },
            _ => bail!("{}", gettext("Unknown mailbox")),
        })
    }
    async fn request(&self, r: &Value) -> Result<Value> {
        let op = string(r, "op");
        match op {
            "notification-settings" | "notification-delivered" | "notification-action" => {
                self.notification_request(r).await
            }
            "accounts" => {
                let accounts = self.accounts().await?;
                let mut items = vec![];
                for a in accounts {
                    let id = a.id;
                    let labels = self.db.read(move |c| labels::list_labels(c, id)).await?;
                    items.push(
                        json!({"account":a,"labels":labels, "connected":self.sync.is_running(id)}),
                    );
                }
                let unread = self
                    .db
                    .read(|c| {
                        threads::unread_threads(
                            c,
                            &threads::ThreadFilter::unified(mailrs_domain::MailSet::Role(
                                mailrs_domain::Role::Inbox,
                            )),
                        )
                    })
                    .await?;
                Ok(json!({"items":items,"unread":unread,"demo":self.demo}))
            }
            "list" => {
                let all = self.accounts().await?;
                let scope = Scope::of_mail_accounts(&all);
                let mailbox = self.mailbox(r)?;
                let limit = r["limit"].as_u64().unwrap_or(500).clamp(1, 2000) as usize;
                let view = View {
                    now: now_millis(),
                    limit: Some(limit),
                    threading: r["threading"].as_bool().unwrap_or(true),
                    ..View::default()
                };
                let listing = self
                    .boxes
                    .list(&mailbox, &scope, &view, Loaded::nothing())
                    .await?;
                Ok(
                    json!({"rows":listing.rows,"title":listing.title,"subtitle":listing.subtitle,"unread":listing.unread,"more":listing.more,"notices":listing.notices}),
                )
            }
            "thread" => {
                let id = account(r);
                let thread = string(r, "thread").to_owned();
                if let Some(qid) = mailrs_sync::outbox_id(&thread) {
                    let q = self
                        .outbox
                        .find(qid)
                        .await?
                        .context(gettext("Queued message not found"))?;
                    let body = mailrs_mime::read(q.raw.as_deref().unwrap_or(&[]));
                    let text = body.text.clone().unwrap_or_default();
                    return Ok(
                        json!({"messages":[{"meta":{"id":q.message_id,"subject":q.subject,"date":q.send_at,"to":[],"from":null},"body":body,"text":text}]}),
                    );
                }
                let sync = self.sync.account(id).ok();
                if let Some(s) = &sync {
                    let _ = s.ensure_thread(&thread).await;
                }
                let metas = self
                    .db
                    .read(move |c| messages::thread_messages(c, id, &thread))
                    .await?;
                let mut parts = vec![];
                for meta in metas {
                    let mid = meta.id.clone();
                    let cached = self
                        .db
                        .read(move |c| mailrs_store::bodies::peek_body(c, id, &mid))
                        .await?;
                    let body = match cached {
                        Some(b) => b,
                        None => match &sync {
                            Some(s) => s.body(&meta.id).await?,
                            None => bail!(
                                "{}",
                                gettext(
                                    "This message is not cached. Reconnect the account to download it."
                                )
                            ),
                        },
                    };
                    let mut result = json!({"body":body,"sealed":false});
                    if pgp::opening(&body).is_some() {
                        let opened = async {
                            let sync = sync.as_ref().context(gettext(
                                "Reconnect the account to read the original protected message.",
                            ))?;
                            let raw = sync.raw_message(&meta.id).await?;
                            crypto::read(self, raw.as_ref().clone(), body.clone()).await
                        }
                        .await;
                        result = match opened {
                            Ok(v) => v,
                            Err(error) => {
                                json!({"body":body,"sealed":false,"mark":{"title":gettext("OpenPGP could not open this message"),"detail":error.to_string(),"tone":"Unchecked"}})
                            }
                        };
                    }
                    let effective: mailrs_domain::MessageBody =
                        serde_json::from_value(result["body"].clone())?;
                    result["text"] =
                        json!(
                            effective
                                .text
                                .unwrap_or_else(|| mailrs_mime::html::html_to_text(
                                    effective.html.as_deref().unwrap_or("")
                                ))
                        );
                    result["meta"] = json!(meta);
                    parts.push(result);
                }
                Ok(json!({"messages":parts}))
            }
            "action" => {
                let targets: Vec<_> = r["targets"]
                    .as_array()
                    .context(gettext("Targets required"))?
                    .iter()
                    .map(|t| Target {
                        account_id: account(t),
                        thread_id: string(t, "thread").into(),
                        message_id: t["message"].as_str().map(str::to_owned),
                    })
                    .collect();
                let action = if string(r, "action") == "remind" {
                    MailAction::Remind {
                        at: r["at"].as_i64().context(gettext("Time required"))?,
                    }
                } else if string(r, "action") == "label" {
                    MailAction::Label {
                        add: vec![string(r, "name").into()],
                        remove: vec![],
                        create: true,
                    }
                } else {
                    MailAction::Triage(
                        string(r, "action")
                            .parse::<TriageAction>()
                            .map_err(anyhow::Error::msg)?,
                    )
                };
                let outcome = self.actions.run(&targets, action, History::Record).await;
                if let Some(error) = outcome.first_error() {
                    bail!("{}", error);
                }
                Ok(json!({"done":outcome.done.len(),"undo":self.actions.newest_words().await}))
            }
            "undo" => {
                if let Some(undone) = self.actions.undo().await {
                    if let Some(error) = undone.outcome.first_error() {
                        bail!("{}", error);
                    }
                    Ok(json!({"done":undone.outcome.done.len()}))
                } else {
                    Ok(json!({"done":0}))
                }
            }
            "refresh" => {
                self.sync.poke_all();
                self.boxes.forget_remote();
                self.outbox.try_now().await?;
                Ok(json!({}))
            }
            "visibility" => {
                self.sync
                    .set_window_open(r["open"].as_bool().unwrap_or(true));
                Ok(json!({}))
            }
            "compose" => compose::submit(self, r).await,
            "crypto-status" | "crypto-keys" | "crypto-import" => crypto::request(self, r).await,
            "draft-open" => {
                use base64::Engine;
                let id = account(r);
                let sync = self.sync.account(id)?;
                let mid = string(r, "message");
                let draft_id = sync
                    .draft_id_for(mid)
                    .await?
                    .context(gettext("This message is no longer a draft"))?;
                let raw = sync.raw_message(mid).await?;
                let parsed = mail_parser::MessageParser::default()
                    .parse(&**raw)
                    .context(gettext("Unreadable draft"))?;
                let addresses = |a: Option<&mail_parser::Address>| {
                    a.map(|a| {
                        a.iter()
                            .filter_map(|v| v.address())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default()
                };
                let original = mailrs_mime::read(&raw);
                let mark = parsed
                    .header("X-Penguin-Mail-Draft")
                    .and_then(|h| h.as_text())
                    .unwrap_or("");
                let encrypt = mark.contains("encrypt")
                    || original.protection == Some(mailrs_domain::Protection::Encrypted);
                let sign = mark.contains("sign");
                let (body, files) = if pgp::opening(&original).is_some() {
                    let result = crypto::read(self, raw.as_ref().clone(), original).await?;
                    if !result["opened"].as_bool().unwrap_or(false) {
                        bail!("{}", result["mark"]["title"].as_str().unwrap_or("OpenPGP"));
                    }
                    let body = serde_json::from_value::<mailrs_domain::MessageBody>(
                        result["body"].clone(),
                    )?;
                    let files = result["files"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|v| {
                                    base64::engine::general_purpose::STANDARD
                                        .decode(v.as_str().unwrap_or(""))
                                })
                                .collect::<std::result::Result<Vec<_>, _>>()
                        })
                        .transpose()?
                        .unwrap_or_default();
                    (body, files)
                } else {
                    (original, mailrs_mime::files(&raw))
                };
                let kept:Vec<_>=body.attachments.iter().zip(files).map(|(a,b)|json!({"name":a.filename,"type":a.mime_type,"data":base64::engine::general_purpose::STANDARD.encode(b)})).collect();
                Ok(
                    json!({"account":id,"draft_id":draft_id,"thread":string(r,"thread"),"subject":parsed.subject().unwrap_or(""),"to":addresses(parsed.to()),"cc":addresses(parsed.cc()),"bcc":addresses(parsed.bcc()),"sign":sign,"encrypt":encrypt,"text":body.text.unwrap_or_else(||mailrs_mime::html::html_to_text(body.html.as_deref().unwrap_or(""))),"kept_files":kept}),
                )
            }

            "outbox-send" => {
                let id = r["queue"]
                    .as_i64()
                    .context(gettext("Queued message required"))?;
                Ok(match self.outbox.send_one(id).await? {
                    mailrs_sync::Posted::Sent(id) => json!({"sent":id}),
                    mailrs_sync::Posted::Waiting(id) => json!({"waiting":id}),
                    mailrs_sync::Posted::Refused(e) => bail!("{}", e),
                })
            }
            "outbox-delete" => {
                self.outbox
                    .drop_one(
                        r["queue"]
                            .as_i64()
                            .context(gettext("Queued message required"))?,
                    )
                    .await?;
                Ok(json!({}))
            }

            "attachment" => {
                let bytes = if let Some(encoded) = r["opened_data"].as_str() {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.decode(encoded)?
                } else {
                    let sync = self.sync.account(account(r))?;
                    sync.attachment(string(r, "message"), string(r, "attachment"))
                        .await?
                };
                let path = std::path::Path::new(string(r, "path"));
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)?;
                file.write_all(&bytes)?;
                Ok(json!({"bytes":bytes.len()}))
            }
            "export" => {
                let sync = self.sync.account(account(r))?;
                let bytes = sync.export_mbox(string(r, "thread"), None).await?;
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(string(r, "path"))?;
                file.write_all(&bytes)?;
                Ok(json!({"bytes":bytes.len()}))
            }
            "add-caldav" | "add-jmap" | "add-pop3" | "remove-account" => {
                account_forms::request(self, r).await
            }
            "oauth-status" => Ok(oauth::status(self)),
            "add-browser" => {
                if self.demo {
                    bail!("{}", gettext("Demo mode cannot add real accounts"));
                }
                let provider = match string(r, "provider") {
                    "google" => Browser::Google,
                    "microsoft" => Browser::Microsoft,
                    _ => bail!("{}", gettext("Unknown provider")),
                };
                let connector = oauth::connector(self, r)?;
                let wanted = if account(r) > 0 {
                    Wanted::Only(
                        self.accounts()
                            .await?
                            .into_iter()
                            .find(|a| a.id == account(r))
                            .context(gettext("Account not found"))?
                            .email,
                    )
                } else {
                    Wanted::Anyone
                };
                let out = self.out.clone();
                let signed = tokio::time::timeout(
                    Duration::from_secs(300),
                    sign_in::in_browser(&connector, provider, wanted, move |url| {
                        emit(&out, json!({"event":"authorize","url":url}));
                    }),
                )
                .await
                .context(gettext("Sign-in timed out"))??;
                self.sync.start_account(signed.account.id, signed.services);
                Ok(json!({"account":signed.account}))
            }
            "add-imap" => {
                if self.demo {
                    bail!("{}", gettext("Demo mode cannot add real accounts"));
                }
                let address = string(r, "address");
                if !address.contains('@') || address.contains(['\r', '\n']) {
                    bail!("{}", gettext("Invalid email address"));
                }
                let imap: mailrs_discover::Server = serde_json::from_value(r["imap"].clone())?;
                let smtp: mailrs_discover::Server = serde_json::from_value(r["smtp"].clone())?;
                let user = r["username"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(address);
                let smtp_user = r["smtp_username"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(user);
                let password = string(r, "password");
                let smtp_password = r["smtp_password"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(password);
                let checked =
                    mailrs_imap::check(&imap, &smtp, user, smtp_user, password, smtp_password)
                        .await?;
                let new = sign_in::NewImap {
                    address: address.into(),
                    provider_name: string(r, "provider_name").into(),
                    servers: mailrs_sync::servers_for(
                        &imap,
                        &checked.imap_user,
                        &smtp,
                        &checked.smtp_user,
                    ),
                    password: password.into(),
                    smtp_password: Some(smtp_password.into()),
                };
                let a = sign_in::imap_signed_in(
                    &self.db,
                    self.connector.secrets.passwords.clone(),
                    new,
                    now_millis(),
                )
                .await?;
                self.connect(&a).await?;
                Ok(json!({"account":a}))
            }
            "discover" => {
                let found =
                    mailrs_discover::find(&mailrs_discover::RealNet::new()?, string(r, "address"))
                        .await;
                let mut candidates = vec![];
                for c in found.candidates {
                    let server = |s: mailrs_discover::Server| json!({"host":s.host,"port":s.port,"security":if matches!(s.security,mailrs_discover::Security::Tls){"tls"}else{"start_tls"},"user_name":"address"});
                    candidates.push(json!({"imap":c.imap.map(server),"smtp":server(c.smtp),"confirm":c.confirm,"provider":c.provider.map(|p|p.name)}));
                }
                Ok(json!({"candidates":candidates}))
            }
            "calendar" => {
                let all = self.accounts().await?;
                let ids: Vec<_> = all
                    .iter()
                    .filter(|a| account(r) == 0 || a.id == account(r))
                    .map(|a| a.id)
                    .collect();
                let from = r["from"].as_i64().unwrap_or(now_millis() - 86400000);
                let to = r["to"].as_i64().unwrap_or(from + 42 * 86400000);
                let mut events = vec![];
                let mut calendars = vec![];
                for id in ids {
                    if !self.sync.is_running(id) {
                        continue;
                    }
                    let list = self
                        .db
                        .read(move |c| mailrs_store::calendar::calendars(c, id))
                        .await?;
                    for c in list {
                        calendars.push(json!({"account":id,"calendar":c}));
                    }
                    for event in permitted(self.calendar.events(id, from, to).await?)? {
                        events.push(json!({"account":id,"occurrence":{"start":event.start,"end":event.end,"id":event.id(),"event":*event.event}}));
                    }
                }
                events.sort_by_key(|v| v["occurrence"]["start"].as_i64());
                Ok(json!({"events":events,"calendars":calendars}))
            }
            "event-save" => {
                let edit = mailrs_domain::calendar::EventEdit {
                    title: Some(string(r, "title").into()),
                    start: r["start"].as_i64(),
                    end: r["end"].as_i64(),
                    place: Some(string(r, "place").into()),
                    description: Some(string(r, "description").into()),
                    ..Default::default()
                };
                if edit.title.as_deref().unwrap_or("").trim().is_empty() || edit.end <= edit.start {
                    bail!("{}", gettext("Enter a title and an end after the start"));
                }
                let event = if string(r, "event").is_empty() {
                    permitted(
                        self.calendar
                            .create(account(r), r["calendar"].as_str(), &edit)
                            .await?,
                    )?
                } else {
                    permitted(
                        self.calendar
                            .update(account(r), string(r, "event"), &edit)
                            .await?,
                    )?
                };
                Ok(json!({"event":event}))
            }
            "event-delete" => {
                permitted(self.calendar.delete(account(r), string(r, "event")).await?)?;
                Ok(json!({}))
            }
            "contacts" => {
                let items = self.db.read(mailrs_store::contacts::suggestions).await?;
                Ok(
                    json!({"items":items.iter().map(|c|json!({"name":c.name,"email":c.email,"organization":c.organization,"accounts":c.accounts})).collect::<Vec<_>>()}),
                )
            }
            "contact-save" => {
                let fields = mailrs_gmail::ContactFields {
                    name: Some(string(r, "name").into()),
                    emails: Some(vec![string(r, "email").into()]),
                    ..Default::default()
                };
                permitted(self.contacts.create(account(r), &fields, true).await?)?;
                Ok(json!({}))
            }
            "settings" => Ok(json!({"sync":self.connector.config.sync})),
            "settings-save" => {
                let mut c = self.connector.config.clone();
                let poll = r["poll_seconds"].as_u64().unwrap_or(30).clamp(15, 3600);
                let days = r["window_days"].as_i64().unwrap_or(30).clamp(1, 3650);
                c.sync.poll_seconds = Some(poll);
                c.sync.window_days = Some(days);
                if !self.demo {
                    c.save(&config::config_path()?)?;
                }
                Ok(json!({"restart":true}))
            }
            _ => bail!("{}", gettext("Unknown request")),
        }
    }
}
fn main() -> Result<()> {
    // No runtime or worker threads exist while the process locale is set.
    unsafe {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(mailrs_sync::WORKER_STACK)
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<()> {
    if let Ok(path) = std::env::var("PENGUIN_MAIL_LOCALEDIR") {
        let _ = gettextrs::bindtextdomain("penguin-mail", path);
    }
    let _ = gettextrs::bind_textdomain_codeset("penguin-mail", "UTF-8");
    let _ = gettextrs::textdomain("penguin-mail");
    tracing_subscriber::util::SubscriberInitExt::init(mailrs_imap::quiet(
        tracing_subscriber::fmt()
            .with_writer(io::stderr)
            .with_env_filter("warn")
            .finish(),
    ));
    let out = Arc::new(Mutex::new(io::stdout()));
    let is_demo = std::env::args().any(|a| a == "--demo");
    let engine = match Engine::start(is_demo, out.clone()).await {
        Ok(e) => e,
        Err(e) => {
            emit(&out, json!({"event":"fatal","error":e.to_string()}));
            return Err(e);
        }
    };
    emit(&out, json!({"event":"ready","demo":is_demo}));
    tokio::spawn(engine.clone().background());
    let reminder_engine = engine.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(15));
        loop {
            timer.tick().await;
            if let Err(error) = reminder_engine.check_reminders().await {
                tracing::warn!(%error, "could not check calendar reminders");
            }
        }
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            let mut line = vec![];
            match (&mut input)
                .take((MAX_REQUEST + 1) as u64)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() > MAX_REQUEST => break,
                Ok(_) => {
                    if tx.blocking_send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(8));
    while let Some(line) = rx.recv().await {
        let parsed = serde_json::from_slice::<Value>(&line);
        let op = parsed.as_ref().ok().map(|r| string(r, "op")).unwrap_or("");
        let write = if matches!(
            op,
            "action"
                | "undo"
                | "crypto-import"
                | "compose"
                | "add-imap"
                | "add-browser"
                | "add-caldav"
                | "add-jmap"
                | "add-pop3"
                | "remove-account"
                | "notification-settings"
                | "notification-delivered"
                | "notification-action"
                | "event-save"
                | "event-delete"
                | "contact-save"
                | "settings-save"
                | "outbox-send"
                | "outbox-delete"
        ) {
            Some(engine.writes.clone().lock_owned().await)
        } else {
            None
        };
        let permit = permits.clone().acquire_owned().await?;
        let e = engine.clone();
        let out = out.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _write = write;
            let reply = match parsed {
                Ok(r) => match e.request(&r).await {
                    Ok(v) => json!({"id":r["id"],"ok":true,"result":v}),
                    Err(error) => json!({"id":r["id"],"ok":false,"error":error.to_string()}),
                },
                Err(_) => json!({"ok":false,"error":"Invalid JSON request"}),
            };
            emit(&out, reply);
        });
    }
    let _finished = permits.acquire_many(8).await?;
    engine.sync.shutdown();
    if let Some(temporary) = &engine._temporary {
        let _ = std::process::Command::new("gpgconf")
            .arg("--homedir")
            .arg(temporary.path().join("gnupg"))
            .args(["--kill", "gpg-agent"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}
