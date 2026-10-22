// SPDX-License-Identifier: GPL-3.0-or-later
// Shared by the GTK and native FLTK readers.
use std::process::Command;

use mailrs_domain::translate::{fill, gettext};
use mailrs_domain::{MessageBody, Protection};
use mailrs_mime::charset::decode_charset;
use mailrs_pgp::{Pgp, PgpError, Recipient, Signature, Trust, Verdict, inline};

use crate::protection::{
    self, Found, Named, Part, Read, Refusal, Signed, Signer, Standard, Vouched,
};

/// Which call of the engine one message needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// `Pgp::verify`, over the first part of a `multipart/signed`.
    Verify,
    /// `Pgp::decrypt`, over the ciphertext of a `multipart/encrypted`.
    Decrypt,
    /// `Pgp::open_inline`, over armor written into the text.
    Inline,
}

/// Which call `body` needs before it is drawn, in the order RFC 3156 reads
/// a message: the wrapper first, and the text only when there is none.
pub fn opening(body: &MessageBody) -> Option<Opening> {
    match body.protection {
        Some(Protection::Signed) => Some(Opening::Verify),
        Some(Protection::Encrypted) => Some(Opening::Decrypt),
        // S/MIME is the other engine's work, and `protection::engine` is
        // where a message goes to find out which of the two it needs.
        Some(_) => None,
        None => inline::armor(body.text.as_deref()?).map(|_| Opening::Inline),
    }
}

/// Runs the call `opening` asks for and says what the window should show.
///
/// `raw` is the message as it arrived, from `format=raw`, which is the one
/// copy whose bytes a signature still covers. Every branch has an answer,
/// including the ones where gpg refused, so the card never goes blank.
pub fn read(pgp: &Pgp, opening: Opening, raw: &[u8], body: &MessageBody) -> Read {
    protection::read(
        Standard::Pgp,
        open(pgp, opening, raw, body),
        body,
        protection::from_address(raw).as_deref(),
    )
}

/// What gpg found in the message, before anything is worded.
pub fn open(pgp: &Pgp, opening: Opening, raw: &[u8], body: &MessageBody) -> Result<Found, Refusal> {
    match opening {
        Opening::Verify => {
            let (part, signature) = protection::wrapper_parts(raw).ok_or(Refusal::Unreadable)?;
            let found = pgp.verify(part, signature).map_err(refusal)?;
            Ok(Found {
                encrypted: false,
                signatures: found.iter().map(signed).collect(),
                part: Part::Entity(part.to_vec()),
            })
        }
        Opening::Decrypt => {
            let (_, ciphertext) = protection::wrapper_parts(raw).ok_or(Refusal::Unreadable)?;
            let opened = pgp.decrypt(ciphertext).map_err(refusal)?;
            Ok(Found {
                encrypted: true,
                signatures: opened.signatures.iter().map(signed).collect(),
                part: Part::Entity(opened.part),
            })
        }
        Opening::Inline => {
            let text = body.text.as_deref().unwrap_or_default();
            let opened = pgp.open_inline(text).map_err(refusal)?;
            Ok(Found {
                // Clearsigned text carries a signature and was never
                // encrypted; armor that opened without one was.
                encrypted: !matches!(inline::armor(text), Some(inline::Armor::Clearsigned)),
                signatures: opened.signatures.iter().map(signed).collect(),
                // The armor said nothing about a character set, so these
                // bytes are read the way a body with no charset is. Text
                // that is UTF-8 already becomes the string without a copy.
                part: Part::Text(
                    String::from_utf8(opened.text)
                        .unwrap_or_else(|err| decode_charset(err.as_bytes(), None)),
                ),
            })
        }
    }
}

/// What gpg says it is, for Preferences. The version is the last word of
/// its first line, as in `gpg (GnuPG) 2.4.8`.
pub fn version(pgp: &Pgp) -> Option<String> {
    let run = Command::new(pgp.program()).arg("--version").output().ok()?;
    protection::version_of(&String::from_utf8_lossy(&run.stdout))
}

/// Why this draft cannot be encrypted under OpenPGP, for the Encrypt
/// button to say. `None` means every recipient has a key, a blind copy's
/// included: gpg leaves that one's key id out of the message.
pub fn cannot_encrypt(held: &[Recipient]) -> Option<String> {
    if held.is_empty() {
        return Some(gettext("Add a recipient whose key gpg holds."));
    }
    let missing: Vec<&str> = held
        .iter()
        .filter(|recipient| recipient.key.is_none())
        .map(|recipient| recipient.address.as_str())
        .collect();
    (!missing.is_empty()).then(|| {
        fill(
            &gettext("gpg holds no key for {addresses}."),
            &[("addresses", &protection::listed(&missing))],
        )
    })
}

/// What Preferences says about the addresses this person sends from:
/// which of them gpg holds a key for, and which it holds nothing for.
pub fn own_keys(held: &[Recipient]) -> String {
    let addresses = |wanted: bool| -> Vec<&str> {
        held.iter()
            .filter(|recipient| recipient.key.is_some() == wanted)
            .map(|recipient| recipient.address.as_str())
            .collect()
    };
    let (mine, missing) = (addresses(true), addresses(false));
    match (mine.as_slice(), missing.as_slice()) {
        ([], _) => gettext("gpg holds no key for any of the addresses you send from."),
        (mine, []) => fill(
            &gettext("gpg holds a key for {addresses}."),
            &[("addresses", &protection::joined(mine))],
        ),
        (mine, missing) => fill(
            &gettext("gpg holds a key for {addresses}, and none for {without}."),
            &[
                ("addresses", &protection::joined(mine)),
                ("without", &protection::joined(missing)),
            ],
        ),
    }
}

/// gpg's answer about a signature, in the words both standards share.
fn signed(signature: &Signature) -> Signed {
    Signed {
        verdict: match signature.verdict {
            Verdict::Good => protection::Verdict::Good,
            Verdict::Bad => protection::Verdict::Bad,
            Verdict::ExpiredKey => protection::Verdict::KeyExpired,
            Verdict::RevokedKey => protection::Verdict::KeyRevoked,
            Verdict::Expired => protection::Verdict::SignatureExpired,
            Verdict::NoKey => protection::Verdict::NoKey,
            Verdict::Unchecked => protection::Verdict::Unchecked,
        },
        signer: signer(signature),
        vouched: vouched(signature.trust),
    }
}

/// Who signed: the name out of the user id gpg reported, and every address
/// on the key, each with the validity gpg gives that user id. A key gpg
/// listed no names for falls back to the one user id in the status line,
/// vouched for as far as the key is.
fn signer(signature: &Signature) -> Signer {
    let key = signature.key_id.clone();
    let reported = signature.signer.as_deref().map(split_user_id);
    let mut addresses: Vec<Named> = signature
        .user_ids
        .iter()
        .filter_map(|named| {
            let (_, address) = split_user_id(&named.user_id);
            Some(Named {
                address: address?,
                vouched: vouched(named.trust),
            })
        })
        .collect();
    if addresses.is_empty()
        && let Some((_, Some(address))) = &reported
    {
        addresses.push(Named {
            address: address.clone(),
            vouched: vouched(signature.trust),
        });
    }
    Signer {
        name: reported.and_then(|(name, _)| name),
        addresses,
        key,
    }
}

/// The name and the address out of a user id, which reads
/// `Ada Lovelace <ada@example.com>`, a bare address, or a bare name.
fn split_user_id(uid: &str) -> (Option<String>, Option<String>) {
    let uid = uid.trim();
    if let Some((name, rest)) = uid.rsplit_once('<')
        && let Some(address) = rest.strip_suffix('>')
    {
        let name = name.trim();
        return (
            (!name.is_empty()).then(|| name.to_string()),
            Some(address.trim().to_string()),
        );
    }
    match uid.contains('@') {
        true => (None, Some(uid.to_string())),
        false => ((!uid.is_empty()).then(|| uid.to_string()), None),
    }
}

/// How far gpg's trust database vouches, in the words both standards share.
fn vouched(trust: Trust) -> Vouched {
    match trust {
        Trust::Ultimate => Vouched::Own,
        Trust::Full => Vouched::Yes,
        Trust::Marginal => Vouched::Partly,
        Trust::Unknown => Vouched::Nobody,
        Trust::Never => Vouched::Never,
    }
}

/// Why gpg would not answer, in the words both standards share.
fn refusal(err: PgpError) -> Refusal {
    match err {
        PgpError::NotForYou => Refusal::NotForYou,
        PgpError::NotPgp => Refusal::Unreadable,
        PgpError::TooLarge => Refusal::TooLarge,
        other => Refusal::Failed(explain(&other)),
    }
}

/// What went wrong with gpg, in the reader's language. `mailrs_pgp` words
/// its errors for a log; anything the window shows goes through here.
pub fn explain(err: &PgpError) -> String {
    match err {
        PgpError::NoGpg => {
            gettext("This computer has no gpg. Install GnuPG to read or send OpenPGP mail.")
        }
        PgpError::CannotRun { program, reason } => fill(
            &gettext("Could not run {program}: {reason}"),
            &[("program", program), ("reason", reason)],
        ),
        PgpError::Temp(reason) => fill(
            &gettext("Could not write a temporary file: {reason}"),
            &[("reason", reason)],
        ),
        PgpError::NotForYou => {
            gettext("This message is encrypted to a key this computer does not hold.")
        }
        PgpError::CannotSign(address) => fill(
            &gettext("gpg holds no secret key to sign as {address}."),
            &[("address", address)],
        ),
        PgpError::NoKeyFor(address) => fill(
            &gettext("gpg holds no key it can encrypt to for {address}."),
            &[("address", address)],
        ),
        PgpError::NotPgp => gettext("This text holds no OpenPGP block."),
        PgpError::NotAKey => gettext("This file holds no OpenPGP key."),
        PgpError::TooLarge => gettext("This message is too large to open."),
        PgpError::Gpg(reason) => fill(&gettext("gpg failed: {reason}"), &[("reason", reason)]),
    }
}
