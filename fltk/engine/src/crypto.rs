// SPDX-License-Identifier: GPL-3.0-or-later
//! The native frontend uses the upstream GnuPG runner and reader verdicts.
use super::*;
use base64::Engine as _;
use mailrs_pgp::{Pgp, Readers};

pub(super) async fn run<T: Send + 'static>(
    e: &Engine,
    work: impl FnOnce(&Pgp) -> Result<T> + Send + 'static,
) -> Result<T> {
    let pgp = e.pgp.clone().context(gettext(
        "This computer has no gpg. Install GnuPG to read or send OpenPGP mail.",
    ))?;
    let permit = e.crypto_gate.clone().acquire_owned().await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(&pgp)
    })
    .await?
}

pub(super) async fn read(
    e: &Engine,
    raw: Vec<u8>,
    body: mailrs_domain::MessageBody,
) -> Result<Value> {
    let opening = pgp::opening(&body);
    let Some(opening) = opening else {
        return Ok(json!({"body":body,"sealed":false}));
    };
    run(e, move |gpg| {
        let read = pgp::read(gpg, opening, &raw, &body);
        let opened = read.body.is_some();
        let files: Vec<_> = read
            .files
            .iter()
            .map(|b| base64::engine::general_purpose::STANDARD.encode(b))
            .collect();
        Ok(
            json!({"body":read.body.unwrap_or(body),"sealed":read.sealed,
            "opened":opened,"files":files,"mark":{"title":read.mark.title,"detail":read.mark.detail,
                "tone":format!("{:?}",read.mark.tone)}}),
        )
    })
    .await
}

pub(super) async fn request(e: &Engine, r: &Value) -> Result<Value> {
    match string(r, "op") {
        "crypto-status" => {
            if e.pgp.is_none() {
                return Ok(json!({"available":false}));
            }
            run(e, |gpg| {
                Ok(json!({"available":true,"version":pgp::version(gpg)}))
            })
            .await
        }
        "crypto-keys" => {
            let from = string(r, "from").to_owned();
            let mut addresses = Vec::new();
            for name in ["to", "cc", "bcc"] {
                for a in compose::recipients(string(r, name))? {
                    if !addresses
                        .iter()
                        .any(|s: &String| s.eq_ignore_ascii_case(&a.email))
                    {
                        addresses.push(a.email);
                    }
                }
            }
            if !from.is_empty() && !addresses.iter().any(|s| s.eq_ignore_ascii_case(&from)) {
                addresses.push(from.clone());
            }
            run(e, move |gpg| {
                let held = gpg.keys_for(&addresses).map_err(|err|anyhow::anyhow!(pgp::explain(&err)))?;
                let keys: Vec<_> = held.iter().map(|r|json!({"address":r.address,"key":r.key.as_ref().map(|k|json!({"fingerprint":k.fingerprint,"user_id":k.user_id,"trust":format!("{:?}",k.trust)}))})).collect();
                Ok(json!({"keys":keys,"can_encrypt":!held.is_empty() && held.iter().all(|r|r.key.is_some()),
                    "cannot_encrypt":pgp::cannot_encrypt(&held),"can_sign":gpg.can_sign(&from).unwrap_or(false)}))
            }).await
        }
        "crypto-import" => {
            let path = string(r, "path");
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 4 * 1024 * 1024 {
                bail!("{}", gettext("Key file exceeds 4 MB"));
            }
            run(e, move |gpg| {
                let found = gpg.import(&bytes).map_err(|err|anyhow::anyhow!(pgp::explain(&err)))?;
                let keys: Vec<_> = found.iter().map(|k|json!({"fingerprint":k.fingerprint,"name":k.name,"address":k.address,"secret":k.secret,"change":format!("{:?}",k.change)})).collect();
                Ok(json!({"keys":keys}))
            }).await
        }
        _ => bail!("{}", gettext("Unknown OpenPGP request")),
    }
}

/// Protect only the MIME entity. Envelope headers stay outside the signature.
pub(super) struct Outgoing {
    pub from: String,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub sign: bool,
    pub encrypt: bool,
    pub draft: bool,
}
pub(super) async fn protect(
    e: &Engine,
    builder: mail_builder::MessageBuilder<'static>,
    outgoing: Outgoing,
) -> Result<Vec<u8>> {
    run(e, move |gpg| encode(gpg, builder, outgoing)).await
}
fn encode(
    gpg: &Pgp,
    builder: mail_builder::MessageBuilder<'static>,
    outgoing: Outgoing,
) -> Result<Vec<u8>> {
    let Outgoing {
        from,
        to,
        cc,
        bcc,
        sign,
        encrypt,
        draft,
    } = outgoing;
    let mut part = Vec::new();
    builder.clone().write_body(&mut part)?;
    let mut envelope = builder;
    envelope.html_body = None;
    envelope.text_body = None;
    envelope.attachments = None;
    let mut named = Vec::new();
    let mut hidden = Vec::new();
    if !draft {
        for a in to.iter().chain(&cc) {
            if !named
                .iter()
                .any(|s: &String| s.eq_ignore_ascii_case(&a.email))
            {
                named.push(a.email.clone());
            }
        }
        for a in &bcc {
            if !named
                .iter()
                .chain(&hidden)
                .any(|s| s.eq_ignore_ascii_case(&a.email))
                && !a.email.eq_ignore_ascii_case(&from)
            {
                hidden.push(a.email.clone());
            }
        }
    }
    if !named.iter().any(|s| s.eq_ignore_ascii_case(&from)) {
        named.push(from.clone());
    }
    let entity = if encrypt {
        let addresses: Vec<_> = named.iter().chain(&hidden).cloned().collect();
        let held = gpg
            .keys_for(&addresses)
            .map_err(|err| anyhow::anyhow!(pgp::explain(&err)))?;
        if let Some(reason) = pgp::cannot_encrypt(&held) {
            bail!("{}", reason);
        }
        // A saved encrypted draft is only for its writer and never prompts to sign.
        gpg.encrypt(
            &part,
            &Readers { named, hidden },
            (sign && !draft).then_some(from.as_str()),
        )
    } else {
        gpg.sign(&part, &from)
    }
    .map_err(|err| anyhow::anyhow!(pgp::explain(&err)))?;
    if draft {
        use mail_builder::headers::raw::Raw;
        envelope = envelope.header(
            "X-Penguin-Mail-Draft",
            Raw::new(if encrypt {
                if sign { "encrypt; sign" } else { "encrypt" }
            } else {
                "sign"
            }),
        );
    }
    Ok(envelope
        .body(mail_builder::mime::MimePart::raw(entity))
        .write_to_vec()?)
}

#[cfg(test)]
mod tests;
