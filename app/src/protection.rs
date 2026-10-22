//! What the two standards share: the words the card speaks, the MIME both
//! read the same way, and which of the two a message needs.
//!
//! `pgp` and `smime` are the adapters, one per standard, and neither has
//! to know the other exists. Each turns what its engine said into a
//! [`Found`], in words neither standard owns, and [`read`] is the one place
//! that turns a `Found` into a card and a body. The body always comes out
//! of the bytes the engine checked or opened, so a part the signature does
//! not cover never shows under the signer's name, whichever standard and
//! whichever shape of message it was.

mod common;
pub mod draft;
pub use common::*;
pub mod remembered;
pub mod run;

use mailrs_domain::translate::{fill, gettext};
use mailrs_domain::{MessageBody, Protection};

use crate::compose::Draft;
use crate::core::Core;
use crate::{pgp, smime};

/// Which engine a message needs, and which of its calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Pgp(pgp::Opening),
    Smime(smime::Opening),
}

/// What each engine holds for a set of addresses. An engine this computer
/// does not have answers nothing at all, which is not the same as holding
/// nothing.
#[derive(Debug, Clone, Default)]
pub struct Held {
    pub pgp: Option<Vec<mailrs_pgp::Recipient>>,
    pub smime: Option<Vec<mailrs_smime::Recipient>>,
}

/// Which call `body` needs before it is drawn, from whichever engine. The
/// wrapper the message arrived in names its standard, and a body with no
/// wrapper is left to OpenPGP, which is the only one of the two that also
/// lives in the text.
pub fn engine(body: &MessageBody) -> Option<Engine> {
    match body.protection {
        Some(Protection::SmimeSigned) => Some(Engine::Smime(smime::Opening::Verify)),
        Some(Protection::SmimeOpaque) => Some(Engine::Smime(smime::Opening::Opaque)),
        Some(Protection::SmimeEnveloped) => Some(Engine::Smime(smime::Opening::Decrypt)),
        _ => pgp::opening(body).map(Engine::Pgp),
    }
}

/// Which standard would encrypt this draft, or what stands in the way.
///
/// OpenPGP wins when both could carry it, so that nothing about a message
/// the app already knew how to send changes the day gpgsm turns up.
/// `blind` says the draft carries a Bcc. OpenPGP keeps a blind copy blind
/// by leaving that reader's key id out of the message ([`Addressees`]);
/// S/MIME names every recipient inside the envelope and has no way not
/// to, so a draft with a Bcc goes out under OpenPGP or not encrypted.
pub fn encrypting(held: &Held, blind: bool) -> Result<Standard, String> {
    let pgp = held.pgp.as_deref().map(pgp::cannot_encrypt);
    let smime = held.smime.as_deref().map(smime::cannot_encrypt);
    match (pgp, smime) {
        (Some(None), _) => Ok(Standard::Pgp),
        (pgp, Some(None)) if blind => Err(match pgp {
            Some(Some(problem)) => fill(
                &gettext("{smime} {pgp}"),
                &[("smime", &smime_names_everyone()), ("pgp", &problem)],
            ),
            _ => smime_names_everyone(),
        }),
        (_, Some(None)) => Ok(Standard::Smime),
        (Some(Some(pgp)), Some(Some(smime))) => Err(neither(
            held.pgp.as_deref().unwrap_or_default(),
            held.smime.as_deref().unwrap_or_default(),
            &pgp,
            &smime,
        )),
        (Some(Some(problem)), None) | (None, Some(Some(problem))) => Err(problem),
        (None, None) => Err(gettext("This computer has nothing to encrypt with.")),
    }
}

/// Why S/MIME will not carry a draft with a Bcc.
pub fn smime_names_everyone() -> String {
    gettext(
        "S/MIME names every recipient inside an encrypted message, so a blind copy would not \
         stay blind.",
    )
}

/// Who an encrypted message goes to, sorted the way the engines need: the
/// people every reader may see, and the blind copies nobody else may.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressees {
    from: String,
    /// To and Cc, each once.
    named: Vec<String>,
    /// Bcc, less anyone already in To or Cc, whom the others see anyway.
    blind: Vec<String>,
}

impl Addressees {
    pub fn of(draft: &Draft) -> Addressees {
        let mut named: Vec<String> = Vec::new();
        for address in draft.to.iter().chain(&draft.cc) {
            add(&mut named, &address.email);
        }
        let mut blind: Vec<String> = Vec::new();
        for address in &draft.bcc {
            if !holds(&named, &address.email) {
                add(&mut blind, &address.email);
            }
        }
        Addressees {
            from: draft.from.email.trim().to_string(),
            named,
            blind,
        }
    }

    /// Whether anyone reads this message on a blind copy.
    pub fn has_blind_copy(&self) -> bool {
        self.blind.iter().any(|email| !same(email, &self.from))
    }

    /// The readers gpg writes the message for. With `own`, gpg holds a key
    /// for the sender, who goes in by name so the copy in Sent stays
    /// readable. A sender who put themselves in Bcc is named too, since
    /// the From line gives them away anyway.
    pub fn readers(&self, own: bool) -> mailrs_pgp::Readers {
        let mut named = self.named.clone();
        let mut hidden = self.blind.clone();
        if own {
            hidden.retain(|email| !same(email, &self.from));
            add(&mut named, &self.from);
        }
        mailrs_pgp::Readers { named, hidden }
    }

    /// The certificates gpgsm envelopes the message for, which it names
    /// one and all. [`encrypting`] keeps a draft with a blind copy away
    /// from S/MIME, so nobody is left out here.
    pub fn certificates(&self, own: bool) -> Vec<String> {
        let readers = self.readers(own);
        let mut all = readers.named;
        for email in readers.hidden {
            add(&mut all, &email);
        }
        all
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn holds(list: &[String], email: &str) -> bool {
    list.iter().any(|held| same(held, email))
}

fn add(list: &mut Vec<String>, email: &str) {
    if !holds(list, email) {
        list.push(email.trim().to_string());
    }
}

/// Which standard signs a message from this address. The sender's own
/// holdings decide it, and OpenPGP wins a tie for the reason
/// [`encrypting`] gives.
pub fn signing(held: &Held) -> Standard {
    let key = held
        .pgp
        .as_deref()
        .is_some_and(|held| held.iter().any(|recipient| recipient.key.is_some()));
    let certificate = held
        .smime
        .as_deref()
        .is_some_and(|held| held.iter().any(|it| it.certificate.is_some()));
    match (key, certificate) {
        (false, true) => Standard::Smime,
        _ => Standard::Pgp,
    }
}

/// What each engine holds for `addresses`. An engine this computer does
/// not have is asked nothing, and one that would not answer is left out
/// the same way, since a question nobody answered is not a missing key.
/// The composer asks this as the recipients change, and the assistant
/// asks it before a message goes out encrypted.
pub async fn held(core: &Core, addresses: &[String]) -> Held {
    let mut held = Held::default();
    if core.has_gpg() {
        let wanted = addresses.to_vec();
        match core.gpg(move |pgp| pgp.keys_for(&wanted)).await {
            Ok(keys) => held.pgp = Some(keys),
            Err(err) => tracing::info!(error = %err, "could not ask gpg about the addresses"),
        }
    }
    if core.has_gpgsm() {
        let wanted = addresses.to_vec();
        match core
            .gpgsm(move |smime| smime.certificates_for(&wanted))
            .await
        {
            Ok(certificates) => held.smime = Some(certificates),
            Err(err) => tracing::info!(error = %err, "could not ask gpgsm about the addresses"),
        }
    }
    held
}

/// Which standard signs a message from `from`, going by what both engines
/// hold of the sender's own.
pub async fn signing_for(core: &Core, from: &str) -> Standard {
    let from = vec![from.trim().to_lowercase()];
    let mut held = held(core, &from).await;
    // Signing needs a secret key. Only gpgsm is asked for one outright:
    // gpg lists a key of the person's own whichever half the question was
    // about.
    if core.has_gpgsm() {
        held.smime = core
            .gpgsm(move |smime| smime.signing_certificates(&from))
            .await
            .ok();
    }
    signing(&held)
}

/// The shield a composer's header shows while its message goes out
/// signed or encrypted: the icon and what it says to the pointer and a
/// screen reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shield {
    pub icon: &'static str,
    pub said: String,
}

/// The shield for a message that goes out signed, encrypted, both, or
/// neither. A plain message shows none: the header keeps the room, and
/// More holds the choices.
pub fn shield(sign: bool, encrypt: bool) -> Option<Shield> {
    let (icon, said) = match (sign, encrypt) {
        (false, false) => return None,
        (true, false) => ("security-high-symbolic", gettext("Signed")),
        (false, true) => ("channel-secure-symbolic", gettext("Encrypted")),
        (true, true) => ("channel-secure-symbolic", gettext("Signed and encrypted")),
    };
    Some(Shield { icon, said })
}

/// What the Encrypt button says once it works, which names the standard
/// the message would go out under rather than making the writer guess.
pub fn encrypting_with(standard: Standard) -> String {
    match standard {
        Standard::Pgp => gettext("Encrypt this message to the recipients' keys"),
        Standard::Smime => gettext("Encrypt this message to the recipients' certificates"),
    }
}

/// What to say when neither standard reaches every recipient. Somebody
/// nothing here can reach is the likelier answer, so that is the one the
/// button gives; a draft that each standard covers half of gets its own
/// sentence, because adding a key would not fix it.
fn neither(
    keys: &[mailrs_pgp::Recipient],
    certificates: &[mailrs_smime::Recipient],
    pgp: &str,
    smime: &str,
) -> String {
    let unreachable: Vec<&str> = keys
        .iter()
        .filter(|recipient| recipient.key.is_none())
        .filter(|recipient| {
            certificates
                .iter()
                .any(|other| other.address == recipient.address && other.certificate.is_none())
        })
        .map(|recipient| recipient.address.as_str())
        .collect();
    if !unreachable.is_empty() {
        return fill(
            &gettext("gpg holds no key and gpgsm no trusted certificate for {addresses}."),
            &[("addresses", &listed(&unreachable))],
        );
    }
    if keys.is_empty() || certificates.is_empty() {
        return pgp.to_string();
    }
    fill(
        &gettext("A message goes out under one standard or the other. {pgp} {smime}"),
        &[("pgp", pgp), ("smime", smime)],
    )
}

#[cfg(test)]
mod tests {
    use mailrs_smime::Certificate;

    use super::*;

    #[test]
    fn a_plain_message_shows_no_shield() {
        assert_eq!(shield(false, false), None);
    }

    #[test]
    fn a_signed_message_shows_the_signing_shield() {
        let shown = shield(true, false).expect("a shield");
        assert_eq!(
            (shown.icon, shown.said.as_str()),
            ("security-high-symbolic", "Signed")
        );
    }

    #[test]
    fn an_encrypted_message_shows_the_lock_whether_or_not_it_is_signed() {
        let encrypted = shield(false, true).expect("a shield");
        assert_eq!(
            (encrypted.icon, encrypted.said.as_str()),
            ("channel-secure-symbolic", "Encrypted")
        );
        let both = shield(true, true).expect("a shield");
        assert_eq!(
            (both.icon, both.said.as_str()),
            ("channel-secure-symbolic", "Signed and encrypted")
        );
    }

    fn certificate(address: &str, held: bool) -> mailrs_smime::Recipient {
        mailrs_smime::Recipient {
            address: address.to_string(),
            certificate: held.then(|| Certificate {
                fingerprint: "F".repeat(40),
                subject: format!("CN={address}"),
                email: address.to_string(),
            }),
        }
    }

    fn key(address: &str, held: bool) -> mailrs_pgp::Recipient {
        mailrs_pgp::Recipient {
            address: address.to_string(),
            key: held.then(|| mailrs_pgp::Key {
                fingerprint: "F".repeat(40),
                user_id: format!("<{address}>"),
                trust: mailrs_pgp::Trust::Unknown,
            }),
        }
    }

    fn arrived(protection: Protection) -> MessageBody {
        MessageBody {
            protection: Some(protection),
            ..MessageBody::default()
        }
    }

    #[test]
    fn the_wrapper_decides_which_engine_a_message_needs() {
        assert_eq!(
            engine(&arrived(Protection::SmimeSigned)),
            Some(Engine::Smime(smime::Opening::Verify))
        );
        assert_eq!(
            engine(&arrived(Protection::SmimeOpaque)),
            Some(Engine::Smime(smime::Opening::Opaque))
        );
        assert_eq!(
            engine(&arrived(Protection::SmimeEnveloped)),
            Some(Engine::Smime(smime::Opening::Decrypt))
        );
        assert_eq!(
            engine(&arrived(Protection::Signed)),
            Some(Engine::Pgp(pgp::Opening::Verify))
        );
        assert_eq!(
            engine(&arrived(Protection::Encrypted)),
            Some(Engine::Pgp(pgp::Opening::Decrypt))
        );
        assert_eq!(engine(&MessageBody::default()), None);
    }

    #[test]
    fn openpgp_carries_the_message_when_both_standards_could() {
        let held = Held {
            pgp: Some(vec![key("ada@example.test", true)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(encrypting(&held, false), Ok(Standard::Pgp));
    }

    #[test]
    fn smime_carries_it_when_it_is_the_one_that_reaches_everybody() {
        let held = Held {
            pgp: Some(vec![key("ada@example.test", false)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(encrypting(&held, false), Ok(Standard::Smime));
    }

    #[test]
    fn a_recipient_neither_standard_reaches_is_named_once() {
        let held = Held {
            pgp: Some(vec![
                key("ada@example.test", true),
                key("bo@example.test", false),
            ]),
            smime: Some(vec![
                certificate("ada@example.test", false),
                certificate("bo@example.test", false),
            ]),
        };
        assert_eq!(
            encrypting(&held, false),
            Err("gpg holds no key and gpgsm no trusted certificate for bo@example.test.".into())
        );
    }

    #[test]
    fn a_draft_each_standard_covers_half_of_says_what_that_means() {
        let held = Held {
            pgp: Some(vec![
                key("ada@example.test", true),
                key("bo@example.test", false),
            ]),
            smime: Some(vec![
                certificate("ada@example.test", false),
                certificate("bo@example.test", true),
            ]),
        };
        let problem = encrypting(&held, false).expect_err("neither reaches both");
        assert!(
            problem.starts_with("A message goes out under one standard or the other."),
            "{problem}"
        );
        assert!(problem.contains("bo@example.test"), "{problem}");
    }

    #[test]
    fn the_only_engine_on_this_computer_is_the_one_that_answers() {
        let smime_alone = Held {
            pgp: None,
            smime: Some(vec![certificate("bo@example.test", false)]),
        };
        assert_eq!(
            encrypting(&smime_alone, false),
            Err(
                "gpgsm holds no trusted certificate for bo@example.test. A certificate that only \
             arrived in mail does not count."
                    .into()
            )
        );
        let pgp_alone = Held {
            pgp: Some(vec![key("bo@example.test", false)]),
            smime: None,
        };
        assert_eq!(
            encrypting(&pgp_alone, false),
            Err("gpg holds no key for bo@example.test.".into())
        );
    }

    #[test]
    fn a_blind_copy_goes_out_under_openpgp_when_it_can() {
        let held = Held {
            pgp: Some(vec![key("ada@example.test", true)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(encrypting(&held, true), Ok(Standard::Pgp));
    }

    #[test]
    fn a_blind_copy_keeps_smime_from_carrying_the_message() {
        let smime_only = Held {
            pgp: Some(vec![key("ada@example.test", false)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        let problem = encrypting(&smime_only, true).expect_err("S/MIME would name the Bcc");
        assert!(
            problem.starts_with("S/MIME names every recipient"),
            "{problem}"
        );
        assert!(
            problem.ends_with("gpg holds no key for ada@example.test."),
            "and says what OpenPGP lacks: {problem}"
        );

        let no_gpg = Held {
            pgp: None,
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(encrypting(&no_gpg, true), Err(smime_names_everyone()));
    }

    fn address(email: &str) -> mailrs_domain::Address {
        mailrs_domain::Address {
            name: None,
            email: email.to_string(),
        }
    }

    fn draft() -> Draft {
        let mut draft = Draft::new(1, address("ada@example.test"));
        draft.to = vec![address("bo@example.test")];
        draft.cc = vec![address("cy@example.test"), address("BO@example.test")];
        draft.bcc = vec![address("di@example.test"), address("cy@example.test")];
        draft
    }

    #[test]
    fn a_blind_copy_is_hidden_and_the_sender_named_when_they_hold_a_key() {
        let addressees = Addressees::of(&draft());
        assert!(addressees.has_blind_copy());
        assert_eq!(
            addressees.readers(true),
            mailrs_pgp::Readers {
                // Cy is in Cc as well as Bcc, so the others see Cy anyway.
                named: vec![
                    "bo@example.test".into(),
                    "cy@example.test".into(),
                    "ada@example.test".into()
                ],
                hidden: vec!["di@example.test".into()],
            }
        );
        // Without a key of the sender's own, the sender is nobody gpg can
        // encrypt to.
        assert_eq!(
            addressees.readers(false).named,
            vec!["bo@example.test".to_string(), "cy@example.test".into()]
        );
    }

    #[test]
    fn a_sender_in_their_own_blind_copy_is_named_rather_than_hidden() {
        let mut draft = draft();
        draft.bcc = vec![address("Ada@example.test")];
        let addressees = Addressees::of(&draft);
        assert!(
            !addressees.has_blind_copy(),
            "the From line names the sender anyway"
        );
        let readers = addressees.readers(true);
        assert!(readers.hidden.is_empty(), "{readers:?}");
        assert!(readers.named.contains(&"ada@example.test".to_string()));
    }

    #[test]
    fn the_sender_decides_which_standard_signs() {
        let both = Held {
            pgp: Some(vec![key("ada@example.test", true)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(signing(&both), Standard::Pgp);

        let certificate_only = Held {
            pgp: Some(vec![key("ada@example.test", false)]),
            smime: Some(vec![certificate("ada@example.test", true)]),
        };
        assert_eq!(signing(&certificate_only), Standard::Smime);

        assert_eq!(signing(&Held::default()), Standard::Pgp);
    }
}
