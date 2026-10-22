// SPDX-License-Identifier: GPL-3.0-or-later
//! Native-adapter checks with a private GnuPG home and no interactive pinentry.
use super::*;
use std::process::{Command, Stdio};
struct Home {
    dir: tempfile::TempDir,
    gpg: Pgp,
}
impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            dir.path().join("gpg-agent.conf"),
            "pinentry-program /bin/false\n",
        )
        .unwrap();
        let gpg = Pgp::find()
            .expect("GnuPG is required for native protection tests")
            .with_home(dir.path());
        assert!(
            Command::new(gpg.program())
                .args(["--batch", "--no-tty", "--homedir"])
                .arg(dir.path())
                .args([
                    "--pinentry-mode",
                    "loopback",
                    "--passphrase",
                    "",
                    "--quick-generate-key",
                    "Alex <alex@example.com>",
                    "future-default",
                    "default",
                    "0"
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        Self { dir, gpg }
    }
    fn signed(&self, sign: bool, encrypt: bool, draft: bool) -> Vec<u8> {
        let builder = mail_builder::MessageBuilder::new()
            .from("alex@example.com")
            .to("alex@example.com")
            .subject("Native protection")
            .text_body("Meet at six.")
            .attachment("text/plain", "note.txt", b"A signed file.".to_vec());
        encode(
            &self.gpg,
            builder,
            Outgoing {
                from: "alex@example.com".into(),
                to: vec![Address {
                    name: None,
                    email: "alex@example.com".into(),
                }],
                cc: vec![],
                bcc: vec![],
                sign,
                encrypt,
                draft,
            },
        )
        .unwrap()
    }
    fn read(&self, raw: &[u8]) -> protection::Read {
        let body = mailrs_mime::read(raw);
        pgp::read(&self.gpg, pgp::opening(&body).unwrap(), raw, &body)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .arg("--homedir")
            .arg(self.dir.path())
            .args(["--kill", "gpg-agent"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
#[test]
fn native_signed_mime_is_verified_without_reencoding() {
    let home = Home::new();
    let raw = home.signed(true, false, false);
    let read = home.read(&raw);
    assert_eq!(read.mark.tone, protection::Tone::Good);
    assert!(read.body.unwrap().text.unwrap().contains("Meet at six."));
    assert_eq!(read.files[0], b"A signed file.");
    assert!(home.gpg.can_sign("alex@example.com").unwrap());
    assert!(!home.gpg.can_sign("absent@example.com").unwrap());
}
#[test]
fn altered_text_fails_verification_and_unsigned_extra_parts_are_never_displayed() {
    let home = Home::new();
    let raw = home.signed(true, false, false);
    let changed = String::from_utf8_lossy(&raw)
        .replace("Meet at six.", "Pay Mallory.")
        .into_bytes();
    assert_eq!(home.read(&changed).mark.tone, protection::Tone::Bad);
    let forged = protection::tampered::with_unsigned_part(&raw);
    let read = home.read(&forged);
    assert_eq!(read.mark.tone, protection::Tone::Good);
    let body = read.body.unwrap();
    assert!(!body.html.unwrap_or_default().contains("Pay Mallory"));
    assert!(!body.attachments.iter().any(|a| a.filename == "invoice.pdf"));
}
#[test]
fn native_encrypted_mail_verifies_inner_signature_and_keeps_attachments() {
    let home = Home::new();
    let raw = home.signed(true, true, false);
    let read = home.read(&raw);
    assert!(read.sealed);
    assert_eq!(read.mark.tone, protection::Tone::Good);
    assert_eq!(read.files[0], b"A signed file.");
    assert!(!String::from_utf8_lossy(&raw).contains("Meet at six."));
    assert!(read.body.unwrap().text.unwrap().contains("Meet at six."));
}
#[test]
fn encrypted_drafts_are_not_signed_until_sending() {
    let home = Home::new();
    let raw = home.signed(true, true, true);
    let read = home.read(&raw);
    assert!(read.sealed);
    assert!(read.body.unwrap().text.unwrap().contains("Meet at six."));
    assert!(String::from_utf8_lossy(&raw).contains("X-Penguin-Mail-Draft: encrypt; sign"));
}
#[test]
fn signer_identity_must_match_the_from_address() {
    let home = Home::new();
    let raw = home.signed(true, false, false);
    let raw = String::from_utf8_lossy(&raw)
        .replacen("From: <alex@example.com>", "From: <mallory@example.net>", 1)
        .into_bytes();
    let parsed = protection::from_address(&raw).unwrap();
    assert_eq!(parsed, "mallory@example.net");
    assert_ne!(home.read(&raw).mark.tone, protection::Tone::Good);
}
#[test]
fn inline_signatures_display_only_the_checked_text() {
    let home = Home::new();
    let mut cmd = Command::new(home.gpg.program());
    cmd.args(["--batch", "--no-tty", "--homedir"])
        .arg(home.dir.path())
        .args(["--clearsign", "--local-user", "alex@example.com"]);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Meet at six.")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let text = format!(
        "Ignore this unsigned text\n{}\nPay Mallory.",
        String::from_utf8_lossy(&result.stdout)
    );
    let body = mailrs_domain::MessageBody {
        text: Some(text),
        ..Default::default()
    };
    let read = pgp::read(&home.gpg, pgp::Opening::Inline, &[], &body);
    let shown = read.body.unwrap().text.unwrap();
    assert!(shown.contains("Meet at six."));
    assert!(!shown.contains("Pay Mallory"));
}
