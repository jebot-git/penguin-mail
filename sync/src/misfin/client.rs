//! TLS is authenticated by explicit endpoint fingerprints, not public CAs.
use super::*;
use openssl::{asn1::Asn1Time, hash::MessageDigest, nid::Nid, pkey::PKey, x509::X509};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
type Stream = tokio_native_tls::TlsStream<tokio::net::TcpStream>;

pub fn hash(bytes: &[u8]) -> Result<String> {
    Ok(openssl::hash::hash(MessageDigest::sha256(), bytes)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub fn identity(certificate: &[u8], key: &[u8]) -> Result<(String, String, Vec<u8>)> {
    ensure!(
        certificate.len() <= 65536 && key.len() <= 65536,
        "{}",
        gettext("Identity file exceeds limit")
    );
    let cert = X509::from_pem(certificate)?;
    // Never let OpenSSL prompt on the engine's private JSON command pipe.
    let private = PKey::private_key_from_pem_callback(key, |_| Ok(0))?;
    ensure!(
        cert.public_key()?.public_eq(&private),
        "{}",
        gettext("Certificate and private key do not match")
    );
    let mailbox = cert
        .subject_name()
        .entries_by_nid(Nid::USERID)
        .next()
        .context(gettext("Identity certificate has no USER_ID mailbox"))?
        .data()
        .to_string()?;
    let names = cert
        .subject_alt_names()
        .context(gettext("Identity certificate has no DNS name"))?;
    let host = names
        .iter()
        .find_map(|n| n.dnsname())
        .context(gettext("Identity certificate has no DNS name"))?;
    let address = wire::Destination::parse(&format!("{mailbox}@{host}"), false)?
        .uri
        .trim_start_matches("misfin://")
        .to_string();
    Ok((
        address,
        hash(&cert.to_der()?)?,
        private.private_key_to_pem_pkcs8()?,
    ))
}
/// Generate a self-signed mailbox certificate without writing a loose key file.
pub fn create_identity(address: &str, name: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    use openssl::{
        bn::{BigNum, MsbOption},
        rsa::Rsa,
        x509::{
            X509NameBuilder,
            extension::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName},
        },
    };
    let (mailbox, host) = address
        .trim()
        .split_once('@')
        .context(gettext("Enter a mailbox@hostname address"))?;
    let host = match url::Host::parse(host)? {
        url::Host::Domain(host) => host.to_ascii_lowercase(),
        _ => bail!("{}", gettext("Identity requires a DNS hostname")),
    };
    ensure!(
        host.len() <= 253
            && host.split('.').all(|label| !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
        "{}",
        gettext("Identity requires a DNS hostname")
    );
    ensure!(
        !mailbox.is_empty()
            && mailbox.len() <= 64
            && mailbox
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)),
        "{}",
        gettext("Invalid mailbox name")
    );
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control),
        "{}",
        gettext("Enter an identity name of 1 to 64 characters")
    );
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;
    let mut subject = X509NameBuilder::new()?;
    subject.append_entry_by_nid(Nid::USERID, mailbox)?;
    subject.append_entry_by_nid(Nid::COMMONNAME, name)?;
    let subject = subject.build();
    let mut certificate = X509::builder()?;
    certificate.set_version(2)?;
    let mut serial = BigNum::new()?;
    serial.rand(128, MsbOption::ONE, false)?;
    certificate.set_serial_number(serial.to_asn1_integer()?.as_ref())?;
    certificate.set_subject_name(&subject)?;
    certificate.set_issuer_name(&subject)?;
    certificate.set_pubkey(&key)?;
    certificate.set_not_before(Asn1Time::days_from_now(0)?.as_ref())?;
    certificate.set_not_after(Asn1Time::days_from_now(1825)?.as_ref())?;
    certificate.append_extension(BasicConstraints::new().critical().build()?)?;
    certificate.append_extension(
        KeyUsage::new()
            .critical()
            .digital_signature()
            .key_encipherment()
            .build()?,
    )?;
    certificate.append_extension(ExtendedKeyUsage::new().client_auth().build()?)?;
    let san = SubjectAlternativeName::new()
        .dns(&host)
        .build(&certificate.x509v3_context(None, None))?;
    certificate.append_extension(san)?;
    certificate.sign(&key, MessageDigest::sha256())?;
    Ok((
        certificate.build().to_pem()?,
        key.private_key_to_pem_pkcs8()?,
    ))
}
pub(super) async fn connect(
    record: &Identity,
    d: &wire::Destination,
    expected: Option<&str>,
) -> Result<(Stream, String)> {
    let identity =
        native_tls::Identity::from_pkcs8(record.certificate.as_bytes(), record.key.as_bytes())?;
    let connector = native_tls::TlsConnector::builder()
        .identity(identity)
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        // Misfin uses self-signed identities. The peer is checked below before
        // writing a request; neither SMTP nor the other providers use this.
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()?;
    let socket = tokio::net::TcpStream::connect((d.host.as_str(), d.port)).await?;
    let stream = tokio_native_tls::TlsConnector::from(connector)
        .connect(&d.host, socket)
        .await?;
    let der = stream
        .get_ref()
        .peer_certificate()?
        .context(gettext("Server supplied no certificate"))?
        .to_der()?;
    let cert = X509::from_der(&der)?;
    let now = Asn1Time::days_from_now(0)?;
    ensure!(
        cert.not_before() <= now && cert.not_after() >= now,
        "{}",
        gettext("Server certificate is not currently valid")
    );
    let names = cert
        .subject_alt_names()
        .context(gettext("Server certificate has no DNS name"))?;
    ensure!(
        names.iter().any(|n| n
            .dnsname()
            .is_some_and(|name| name.eq_ignore_ascii_case(&d.host))),
        "{}",
        gettext("Server certificate does not name this host")
    );
    let fingerprint = hash(&der)?;
    if let Some(pin) = expected {
        ensure!(
            fingerprint == pin,
            "{}",
            gettext("Server fingerprint changed; no request was sent")
        );
    }
    Ok((stream, fingerprint))
}
pub async fn probe(record: &Identity, d: &wire::Destination) -> Result<String> {
    let (_, fingerprint) = connect(record, d, None).await?;
    Ok(fingerprint)
}
async fn response(stream: &mut Stream) -> Result<(u16, String)> {
    let mut line = vec![];
    loop {
        ensure!(
            line.len() < 2048,
            "{}",
            gettext("Response exceeds protocol limit")
        );
        let mut byte = [0];
        stream.read_exact(&mut byte).await?;
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&line[..line.len() - 2])?;
    ensure!(
        text.len() >= 2 && text.as_bytes()[..2].iter().all(u8::is_ascii_digit),
        "{}",
        gettext("Invalid response status")
    );
    ensure!(
        text.len() == 2 || text.as_bytes()[2] == b' ',
        "{}",
        gettext("Invalid response separator")
    );
    Ok((text[..2].parse()?, text.get(3..).unwrap_or("").into()))
}
pub async fn send(
    mut stream: Stream,
    d: &wire::Destination,
    message: &str,
) -> Result<(u16, String)> {
    let bytes = wire::request(d, message)?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    response(&mut stream).await
}
pub async fn fetch(mut stream: Stream, d: &wire::Destination) -> Result<Vec<u8>> {
    stream
        .write_all(format!("{}\r\n", d.uri).as_bytes())
        .await?;
    stream.flush().await?;
    let (status, meta) = response(&mut stream).await?;
    ensure!(
        status == 20,
        "{} {status}: {meta}",
        gettext("Gemini server declined the mailbox request")
    );
    let mime = meta.split(';').next().unwrap_or("").trim();
    ensure!(
        matches!(
            mime,
            "text/plain"
                | "text/gemini"
                | "application/gembox"
                | "text/gemmail"
                | "application/gemmail"
        ),
        "{}",
        gettext("Server did not return a text mailbox archive")
    );
    let mut bytes = vec![];
    stream
        .take(wire::MAX_ARCHIVE as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() <= wire::MAX_ARCHIVE,
        "{}",
        gettext("Mailbox archive exceeds 4 MiB")
    );
    Ok(bytes)
}
