//! Misfin(C) framing and gemmail. No MIME or SMTP interpretation is involved.
use super::*;

pub const MAX_BODY: usize = 16_384;
pub const MAX_ARCHIVE: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct Destination {
    pub uri: String,
    pub host: String,
    pub port: u16,
    pub peer: String,
}
impl Destination {
    pub fn parse(input: &str, gemini: bool) -> Result<Self> {
        let scheme = if gemini { "gemini" } else { "misfin" };
        let input = input.trim();
        ensure!(
            !input.chars().any(char::is_control),
            "{}",
            gettext("Invalid destination")
        );
        let text = if !gemini && !input.contains("://") {
            format!("misfin://{input}")
        } else {
            input.into()
        };
        let mut url = url::Url::parse(&text)?;
        ensure!(
            url.scheme() == scheme && url.fragment().is_none() && url.password().is_none(),
            "{}",
            gettext("Invalid destination")
        );
        let raw = url
            .host_str()
            .context(gettext("Missing server hostname"))?
            .trim_matches(['[', ']']);
        let host = match url::Host::parse(raw)? {
            url::Host::Domain(domain) => domain.to_ascii_lowercase(),
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => ip.to_string(),
        };
        let normalized = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.clone()
        };
        url.set_host(Some(&normalized))
            .map_err(|_| anyhow::anyhow!(gettext("Missing server hostname")))?;
        let port = url.port().unwrap_or(if gemini { 1965 } else { 1958 });
        ensure!(port != 0, "{}", gettext("Invalid port"));
        if !gemini {
            ensure!(
                !url.username().is_empty()
                    && url.query().is_none()
                    && matches!(url.path(), "" | "/"),
                "{}",
                gettext("Invalid Misfin address")
            );
            ensure!(
                url.username()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)),
                "{}",
                gettext("Invalid mailbox name")
            );
        } else {
            ensure!(
                url.username().is_empty(),
                "{}",
                gettext("Gemini URL must not contain a user name")
            );
        }
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let uri = if gemini {
            url.to_string()
        } else {
            let suffix = if port == 1958 {
                String::new()
            } else {
                format!(":{port}")
            };
            let domain = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.clone()
            };
            format!("misfin://{}@{domain}{suffix}", url.username())
        };
        ensure!(
            uri.len() + 16 <= 1024,
            "{}",
            gettext("Destination exceeds protocol limit")
        );
        Ok(Self {
            uri,
            host,
            port,
            peer: format!("{scheme}://{authority}"),
        })
    }
}
pub fn compose(subject: &str, body: &str) -> Result<String> {
    ensure!(
        !subject.chars().any(char::is_control),
        "{}",
        gettext("Subject must occupy one line")
    );
    ensure!(
        !body.contains('\0'),
        "{}",
        gettext("Message contains a null byte")
    );
    // The receiving server adds the sender and receipt timestamp. The three
    // metadata lines must still exist in a newly composed Misfin(C) message.
    let text = format!(
        "\n\n\n# {}\n\n{}",
        subject.trim(),
        body.replace("\r\n", "\n").replace('\r', "\n")
    );
    ensure!(
        text.len() <= MAX_BODY,
        "{}",
        gettext("Misfin(C) messages cannot exceed 16384 UTF-8 bytes")
    );
    Ok(text)
}
pub fn request(destination: &Destination, message: &str) -> Result<Vec<u8>> {
    ensure!(
        message.len() <= MAX_BODY,
        "{}",
        gettext("Message exceeds protocol limit")
    );
    let header = format!("{}\t{}\r\n", destination.uri, message.len());
    ensure!(
        header.len() <= 1024,
        "{}",
        gettext("Request header exceeds protocol limit")
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(message.as_bytes());
    Ok(bytes)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub identity: String,
    pub direction: String,
    pub from: String,
    pub to: String,
    pub timestamp: String,
    pub subject: String,
    pub body: String,
}
pub fn archive(bytes: &[u8], identity: &str) -> Result<Vec<Message>> {
    ensure!(
        bytes.len() <= MAX_ARCHIVE,
        "{}",
        gettext("Mailbox archive exceeds 4 MiB")
    );
    let text = std::str::from_utf8(bytes)?.replace("\r\n", "\n");
    let mut messages = vec![];
    for entry in text.split("<=====\n").filter(|s| !s.trim().is_empty()) {
        ensure!(
            entry.len() <= 65536,
            "{}",
            gettext("Gemmail entry exceeds limit")
        );
        let mut lines = entry.splitn(4, '\n');
        let from = lines.next().context(gettext("Missing senders"))?;
        let to = lines.next().context(gettext("Missing recipients"))?;
        let timestamp = lines.next().context(gettext("Missing timestamp"))?;
        let body = lines
            .next()
            .context(gettext("Expected Misfin(C) gemmail metadata"))?;
        let subject = body
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .unwrap_or("");
        messages.push(Message {
            id: hash(entry.as_bytes())?,
            identity: identity.into(),
            direction: "received".into(),
            from: from.into(),
            to: to.into(),
            timestamp: timestamp.into(),
            subject: subject.into(),
            body: body.into(),
        });
        ensure!(
            messages.len() <= 500,
            "{}",
            gettext("Mailbox archive contains too many messages")
        );
    }
    Ok(messages)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_uses_utf8_byte_length_and_preserves_newlines() {
        let d = Destination::parse("bob@EXAMPLE.org", false).unwrap();
        let message = compose("Žluťoučký", "Hello\r\n世界").unwrap();
        let bytes = request(&d, &message).unwrap();
        assert!(
            bytes
                .starts_with(format!("misfin://bob@example.org\t{}\r\n", message.len()).as_bytes())
        );
        assert!(message.starts_with("\n\n\n# Žluťoučký\n"));
        assert!(compose("bad\r\nsubject", "x").is_err());
        assert!(compose("x", &"界".repeat(6000)).is_err());
    }
    #[test]
    fn addresses_cannot_inject_headers_or_change_protocol() {
        for bad in [
            "bob@host\t0\r\n",
            "https://bob@host",
            "bob:p@host",
            "bob@host/path",
            "bob@host?q=x",
            "bob@host#x",
            "@host",
        ] {
            assert!(Destination::parse(bad, false).is_err(), "{bad}");
        }
        assert_eq!(
            Destination::parse("bob@localhost:1234", false)
                .unwrap()
                .port,
            1234
        );
        assert!(Destination::parse("gemini://user@host/", true).is_err());
    }
    #[test]
    fn imports_c_archives_without_interpreting_message_markup() {
        let data=b"alice@one Alice\nbob@two\n2026-10-24T12:00:00Z\n# Hello\n<body>\n<=====\ncarol@three\nbob@two\n2026-10-24T12:01:00Z\n# Next\nBye";
        let entries = archive(data, "id").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].subject, "Hello");
        assert!(entries[0].body.contains("<body>"));
        assert!(archive(b"not gemmail", "id").is_err());
    }
}
