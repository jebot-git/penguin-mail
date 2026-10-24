//! Optional Misfin(C) identities, explicit TLS trust, delivery and local history.
mod client;
mod wire;
use anyhow::{Context, Result, bail, ensure};
use client::hash;
use mailrs_domain::translate::gettext;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use wire::{Destination, Message};

#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    id: String,
    address: String,
    certificate: String,
    key: String,
    #[serde(default)]
    inbox_url: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct State {
    identities: Vec<Identity>,
    pins: BTreeMap<String, String>,
    #[serde(default)]
    recipients: BTreeMap<String, String>,
    messages: Vec<Message>,
}
/// Owns a private store separate from the email schema and serializes only
/// Misfin work, so a slow Misfin server cannot stall normal mail actions.
pub struct Misfin {
    directory: PathBuf,
    state: tokio::sync::Mutex<State>,
}
impl Misfin {
    pub fn open(directory: PathBuf) -> Result<Self> {
        crate::config::private_dir(&directory)?;
        let path = directory.join("state.json");
        let state = if path.exists() {
            serde_json::from_slice(&read(&path, 8 * 1024 * 1024)?)?
        } else {
            State::default()
        };
        Ok(Self {
            directory,
            state: tokio::sync::Mutex::new(state),
        })
    }
    fn save(&self, state: &State) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        let bytes = serde_json::to_vec(state)?;
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "{}",
            gettext("Misfin storage exceeds 8 MiB")
        );
        file.as_file_mut().write_all(&bytes)?;
        file.as_file_mut().flush()?;
        file.as_file().sync_all()?;
        file.persist(self.directory.join("state.json"))?;
        Ok(())
    }
    pub async fn request(&self, r: &serde_json::Value) -> Result<serde_json::Value> {
        use serde_json::json;
        let mut request = r.clone();
        if matches!(r["op"].as_str(), Some("compose" | "misfin-check-compose")) {
            validate_compose(r)?;
            wire::compose(
                r["subject"].as_str().unwrap_or(""),
                r["text"].as_str().unwrap_or(""),
            )?;
            Destination::parse(r["to"].as_str().unwrap_or(""), false)?;
            request["op"] = json!(if r["op"] == "misfin-check-compose" {
                "misfin-checked"
            } else {
                "misfin-send"
            });
            request["destination"] = r["to"].clone();
            request["body"] = r["text"].clone();
        }
        let r = &request;
        let mut state = self.state.lock().await;
        let text = |key: &str| r[key].as_str().unwrap_or("");
        if matches!(
            text("op"),
            "misfin-import-identity" | "misfin-create-identity"
        ) {
            let creating = text("op") == "misfin-create-identity";
            let (certificate, key) = if creating {
                ensure!(
                    state.identities.len() < 16,
                    "{}",
                    gettext("At most 16 Misfin identities can be imported")
                );
                let address = text("address").to_owned();
                let name = text("name").to_owned();
                tokio::task::spawn_blocking(move || client::create_identity(&address, &name))
                    .await??
            } else {
                (
                    read(Path::new(text("certificate")), 65536)?,
                    read(Path::new(text("key")), 65536)?,
                )
            };
            let (address, id, key) = client::identity(&certificate, &key)?;
            ensure!(
                !creating || !state.identities.iter().any(|i| i.address == address),
                "{}",
                gettext(
                    "An identity for this address already exists. Import its certificate instead."
                )
            );
            if !text("inbox_url").is_empty() {
                Destination::parse(text("inbox_url"), true)?;
            }
            let mut next = state.clone();
            let record = Identity {
                id: id.clone(),
                address,
                certificate: String::from_utf8(certificate)?,
                key: String::from_utf8(key)?,
                inbox_url: text("inbox_url").into(),
            };
            if let Some(old) = next.identities.iter_mut().find(|i| i.id == id) {
                let inbox_url = if r["inbox_url"].is_string() {
                    record.inbox_url.clone()
                } else {
                    old.inbox_url.clone()
                };
                *old = record;
                old.inbox_url = inbox_url;
            } else {
                ensure!(
                    state.identities.len() < 16,
                    "{}",
                    gettext("At most 16 Misfin identities can be imported")
                );
                next.identities.push(record);
            }
            self.save(&next)?;
            *state = next;
            return Ok(
                json!({"identity":id,"account":account_number(state.identities.iter().find(|i|i.id==id).unwrap())?}),
            );
        }
        if text("op") == "misfin-accounts" {
            let mut ids = std::collections::BTreeSet::new();
            let mut items = vec![];
            for identity in &state.identities {
                let id = account_number(identity)?;
                ensure!(
                    ids.insert(id),
                    "{}",
                    gettext("Misfin account identifiers conflict")
                );
                items.push(json!({"account":{"id":id,"email":identity.address,"provider":"Misfin",
                    "state":"Active","misfin_identity":identity.id,"misfin_inbox_url":identity.inbox_url},
                    "labels":[],"connected":true}));
            }
            return Ok(json!({"items":items}));
        }
        if text("op") == "misfin-list" {
            let account = r["account"].as_i64().unwrap_or(0);
            let mailbox = text("mailbox");
            ensure!(
                matches!(mailbox, "inbox" | "sent" | "all" | "search"),
                "{}",
                gettext("This mailbox is not available for Misfin")
            );
            let mut rows = vec![];
            let query = text("query").to_lowercase();
            for identity in &state.identities {
                let id = account_number(identity)?;
                if account != 0 && id != account {
                    continue;
                }
                for m in state.messages.iter().filter(|m| m.identity == identity.id) {
                    if (mailbox == "inbox" && m.direction != "received")
                        || (mailbox == "sent" && m.direction != "sent")
                    {
                        continue;
                    }
                    if mailbox == "search"
                        && !format!("{} {} {} {}", m.from, m.to, m.subject, m.body)
                            .to_lowercase()
                            .contains(&query)
                    {
                        continue;
                    }
                    let date = chrono::DateTime::parse_from_rfc3339(&m.timestamp)
                        .map(|d| d.timestamp_millis())
                        .unwrap_or(0);
                    rows.push(
                        json!({"id":m.id,"account_id":id,"message_id":m.id,"from":m.from,
                        "subject":m.subject,"snippet":m.body.chars().take(160).collect::<String>(),
                        "last_message_at":date,"unread":false,"starred":false,"protocol":"misfin"}),
                    );
                }
            }
            rows.sort_by_key(|row| std::cmp::Reverse(row["last_message_at"].as_i64().unwrap_or(0)));
            let limit = r["limit"].as_u64().unwrap_or(500).clamp(1, 2000) as usize;
            let more = rows.len() > limit;
            rows.truncate(limit);
            return Ok(json!({"rows":rows,"more":more,"unread":0}));
        }
        if text("op") == "misfin-state" {
            return Ok(
                json!({"identities":state.identities.iter().map(|i|json!({"id":i.id,"address":i.address})).collect::<Vec<_>>(),"messages":state.messages}),
            );
        }
        let record = state
            .identities
            .iter()
            .find(|i| {
                if text("identity").is_empty() {
                    account_number(i).ok() == r["account"].as_i64()
                } else {
                    i.id == text("identity")
                }
            })
            .context(gettext("Choose a Misfin identity first"))?
            .clone();
        if text("op") == "misfin-checked" {
            return Ok(json!({}));
        }
        if text("op") == "misfin-settings" {
            if !text("inbox_url").is_empty() {
                Destination::parse(text("inbox_url"), true)?;
            }
            let mut next = state.clone();
            next.identities
                .iter_mut()
                .find(|i| i.id == record.id)
                .unwrap()
                .inbox_url = text("inbox_url").into();
            self.save(&next)?;
            *state = next;
            return Ok(json!({}));
        }
        if text("op") == "misfin-thread" {
            let m = state
                .messages
                .iter()
                .find(|m| m.identity == record.id && m.id == text("thread"))
                .context(gettext("Misfin message not found"))?;
            let date = chrono::DateTime::parse_from_rfc3339(&m.timestamp)
                .map(|d| d.timestamp_millis())
                .unwrap_or(0);
            return Ok(
                json!({"messages":[{"meta":{"id":m.id,"subject":m.subject,"date":date,
                "from":{"email":m.from.split_whitespace().next().unwrap_or(""),"name":m.from.split_once(' ').map(|(_,name)|name)},"to":[{"email":m.to.split_whitespace().next().unwrap_or("")}],"cc":[],"rfc822_msgid":null},
                "body":{"text":m.body,"attachments":[],"protection":null},"text":m.body,"sealed":false}],"protocol":"misfin"}),
            );
        }
        if text("op") == "misfin-certificate" {
            return Ok(
                json!({"certificate":record.certificate,"fingerprint":record.id,"address":record.address}),
            );
        }
        if text("op") == "misfin-import-mail" {
            let bytes = read(Path::new(text("path")), wire::MAX_ARCHIVE)?;
            merge(&mut state.messages, wire::archive(&bytes, &record.id)?);
            self.save(&state)?;
            return Ok(json!({}));
        }
        let gemini = text("op") == "misfin-fetch" || r["gemini"].as_bool() == Some(true);
        let destination = Destination::parse(text("destination"), gemini)?;
        if text("op") == "misfin-probe" {
            let fingerprint = timed(client::probe(&record, &destination)).await?;
            return Ok(
                json!({"peer":destination.peer,"fingerprint":fingerprint,"trusted":state.pins.get(&destination.peer)==Some(&fingerprint),"previous":state.pins.get(&destination.peer)}),
            );
        }
        let pin = text("fingerprint");
        ensure!(
            pin.len() == 64 && pin.bytes().all(|c| c.is_ascii_hexdigit()),
            "{}",
            gettext("Invalid server fingerprint")
        );
        match text("op") {
            "misfin-send" => {
                let message = wire::compose(text("subject"), text("body"))?;
                let (stream, _) = timed(client::connect(&record, &destination, Some(pin))).await?;
                state.pins.insert(destination.peer.clone(), pin.to_string());
                self.save(&state)?;
                let (status,meta)=timed(client::send(stream,&destination,&message)).await
                    .context(gettext("Delivery was not confirmed. The message may have arrived; check before sending again."))?;
                if status != 20 {
                    bail!(
                        "{} {status}: {meta}",
                        gettext("Misfin server declined the message")
                    );
                }
                let invalid_fingerprint =
                    !(meta.len() == 64 && meta.bytes().all(|b| b.is_ascii_hexdigit()));
                let recipient_changed = state
                    .recipients
                    .get(&destination.uri)
                    .is_some_and(|old| !old.eq_ignore_ascii_case(&meta));
                if !invalid_fingerprint && !recipient_changed {
                    state
                        .recipients
                        .insert(destination.uri.clone(), meta.to_ascii_lowercase());
                }
                let stamp = chrono::Utc::now().to_rfc3339();
                let entry = Message {
                    id: hash(format!("{stamp}{message}").as_bytes())?,
                    identity: record.id,
                    direction: "sent".into(),
                    from: record.address,
                    to: destination.uri.trim_start_matches("misfin://").into(),
                    timestamp: stamp,
                    subject: text("subject").into(),
                    body: text("body").into(),
                };
                merge(&mut state.messages, vec![entry]);
                let warning = if self.save(&state).is_err() {
                    Some(gettext(
                        "Message accepted, but the local copy could not be saved.",
                    ))
                } else if invalid_fingerprint {
                    Some(gettext(
                        "Message accepted, but the recipient fingerprint is invalid. Do not resend.",
                    ))
                } else if recipient_changed {
                    Some(format!(
                        "{}\n{}",
                        gettext(
                            "Message accepted, but the recipient's certificate fingerprint changed. Verify it before sending again."
                        ),
                        meta
                    ))
                } else {
                    None
                };
                Ok(json!({"status":status,"fingerprint":meta,"warning":warning}))
            }
            "misfin-fetch" => {
                let (stream, _) = timed(client::connect(&record, &destination, Some(pin))).await?;
                state.pins.insert(destination.peer.clone(), pin.to_string());
                self.save(&state)?;
                let bytes = timed(client::fetch(stream, &destination)).await?;
                merge(&mut state.messages, wire::archive(&bytes, &record.id)?);
                self.save(&state)?;
                Ok(json!({}))
            }
            _ => bail!("{}", gettext("Unknown Misfin request")),
        }
    }
}
fn merge(history: &mut Vec<Message>, entries: Vec<Message>) {
    for entry in entries {
        if !history
            .iter()
            .any(|m| m.id == entry.id && m.identity == entry.identity)
        {
            history.push(entry);
        }
    }
    let mut bytes: usize = history.iter().map(message_size).sum();
    while history.len() > 500 || bytes > wire::MAX_ARCHIVE {
        bytes -= message_size(&history.remove(0));
    }
}
fn message_size(m: &Message) -> usize {
    m.body.len() + m.subject.len() + m.from.len() + m.to.len() + m.timestamp.len() + 256
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "{}",
        gettext("Expected a regular file")
    );
    let mut bytes = vec![];
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "{}",
        gettext("File exceeds size limit")
    );
    Ok(bytes)
}
async fn timed<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(25), future)
        .await
        .context(gettext("Misfin connection timed out"))?
}

// Native account numbers occupy a separate range from SQLite email accounts.
fn account_number(identity: &Identity) -> Result<i64> {
    Ok(-i64::from_str_radix(
        identity
            .id
            .get(..15)
            .context("Invalid Misfin identity fingerprint")?,
        16,
    )? - 1)
}
fn validate_compose(r: &serde_json::Value) -> Result<()> {
    let text = |key: &str| r[key].as_str().unwrap_or("");
    ensure!(
        r["misfin"].as_bool() == Some(true),
        "{}",
        gettext("Select Misfin when sending from a Misfin account")
    );
    ensure!(
        text("cc").is_empty()
            && text("bcc").is_empty()
            && r["files"].as_array().is_none_or(|v| v.is_empty())
            && r["kept_files"].as_array().is_none_or(|v| v.is_empty())
            && r["draft"].as_bool() != Some(true)
            && r["send_at"].is_null()
            && r["sign"].as_bool() != Some(true)
            && r["encrypt"].as_bool() != Some(true)
            && text("draft_id").is_empty(),
        "{}",
        gettext(
            "Misfin supports one recipient and plain Gemtext, up to 16384 UTF-8 bytes. Cc, Bcc, attachments, drafts, scheduled sending and OpenPGP are unavailable."
        )
    );
    Ok(())
}
