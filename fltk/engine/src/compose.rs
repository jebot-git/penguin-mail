// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use mail_builder::{MessageBuilder, headers::address::Address as MimeAddress};
use mailrs_mime::address::parse_address_list_keeping_invalid;
pub(super) fn recipients(text: &str) -> Result<Vec<Address>> {
    let addresses = parse_address_list_keeping_invalid(text);
    if addresses
        .iter()
        .any(|a| !a.email.contains('@') || a.email.contains(['\r', '\n']))
    {
        bail!("{}", gettext("Invalid recipient address"));
    }
    Ok(addresses)
}
fn mime_addresses(items: &[Address]) -> Vec<MimeAddress<'static>> {
    items
        .iter()
        .map(|a| MimeAddress::new_address(a.name.clone(), a.email.clone()))
        .collect()
}
pub(super) async fn submit(e: &Engine, r: &Value) -> Result<Value> {
    let id = account(r);
    let all = e.accounts().await?;
    let from = all
        .iter()
        .find(|a| a.id == id)
        .context(gettext("Choose a sending account"))?;
    let to = recipients(string(r, "to"))?;
    let cc = recipients(string(r, "cc"))?;
    let bcc = recipients(string(r, "bcc"))?;
    let draft = r["draft"].as_bool().unwrap_or(false);
    if !draft && to.is_empty() && cc.is_empty() && bcc.is_empty() {
        bail!("{}", gettext("Enter a recipient"));
    }
    let subject = string(r, "subject");
    if subject.contains(['\r', '\n']) {
        bail!("{}", gettext("Subject cannot contain a line break"));
    }
    let mut builder = MessageBuilder::new()
        .from(from.email.clone())
        .to(mime_addresses(&to))
        .cc(mime_addresses(&cc))
        .bcc(mime_addresses(&bcc))
        .subject(subject.to_owned())
        .text_body(string(r, "text").to_owned());
    if let Some(reply) = r["in_reply_to"]
        .as_str()
        .filter(|s| !s.contains(['\r', '\n']))
    {
        builder = builder.in_reply_to(reply.to_owned());
    }
    let mut size = 0;
    if let Some(files) = r["files"].as_array() {
        for file in files {
            let path =
                std::path::Path::new(file.as_str().context(gettext("Invalid attachment path"))?);
            let length = std::fs::metadata(path)?.len();
            size += length;
            if size > 25 * 1024 * 1024 {
                bail!("{}", gettext("Attachments exceed 25 MB"));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(25 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 25 * 1024 * 1024 {
                bail!("{}", gettext("Attachments exceed 25 MB"));
            }
            builder = builder.attachment(
                "application/octet-stream",
                path.file_name()
                    .context(gettext("Attachment name missing"))?
                    .to_string_lossy()
                    .into_owned(),
                bytes,
            );
        }
    }
    if let Some(files) = r["kept_files"].as_array() {
        use base64::Engine;
        for file in files {
            let bytes = base64::engine::general_purpose::STANDARD.decode(string(file, "data"))?;
            size += bytes.len() as u64;
            if size > 25 * 1024 * 1024 {
                bail!("{}", gettext("Attachments exceed 25 MB"));
            }
            builder = builder.attachment(
                string(file, "type").to_owned(),
                string(file, "name").to_owned(),
                bytes,
            );
        }
    }
    let sign = r["sign"].as_bool().unwrap_or(false);
    let encrypt = r["encrypt"].as_bool().unwrap_or(false);
    let raw = if sign || encrypt {
        crypto::protect(
            e,
            builder,
            crypto::Outgoing {
                from: from.email.clone(),
                to: to.clone(),
                cc,
                bcc,
                sign,
                encrypt,
                draft,
            },
        )
        .await?
    } else {
        builder.write_to_vec()?
    };
    let thread = r["thread"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if draft {
        let saved = e
            .sync
            .account(id)?
            .save_draft(raw, thread, r["draft_id"].as_str().map(str::to_owned))
            .await?;
        return Ok(
            json!({"draft_id":saved.draft_id,"message_id":saved.message_id,"thread":saved.thread_id}),
        );
    }
    let queued = mailrs_store::outbox::Queued {
        account_id: id,
        thread_id: thread,
        subject: subject.into(),
        recipients: to
            .iter()
            .map(|a| a.email.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        raw: Some(raw),
        composer: serde_json::to_string(r)?,
        send_at: r["send_at"].as_i64().unwrap_or(now_millis()),
        ..Default::default()
    };
    let posted = if queued.send_at > now_millis() {
        e.outbox.schedule(queued).await?
    } else {
        e.outbox.post(queued).await?
    };
    Ok(match posted {
        mailrs_sync::Posted::Sent(id) => json!({"sent":true,"message":id}),
        mailrs_sync::Posted::Waiting(id) => json!({"queued":true,"id":id}),
        mailrs_sync::Posted::Refused(reason) => bail!("{}", reason),
    })
}
