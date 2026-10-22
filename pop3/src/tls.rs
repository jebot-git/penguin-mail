//! TLS for POP3: TLS 1.2 or 1.3 with platform roots and hostname checks.
//! Platform TLS also supports providers that still require RSA TLS 1.2 suites.

use native_tls::{Protocol, TlsConnector, TlsConnectorBuilder};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_native_tls::TlsStream;

use crate::Pop3Error;

fn builder() -> TlsConnectorBuilder {
    let mut builder = TlsConnector::builder();
    builder.min_protocol_version(Some(Protocol::Tlsv12));
    builder
}

pub(crate) async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    host: &str,
    stream: S,
) -> Result<TlsStream<S>, Pop3Error> {
    let connector = builder().build().map_err(|err| failed(host, err))?;
    handshake_with(host, stream, connector).await
}

fn failed(host: &str, error: impl std::fmt::Display) -> Pop3Error {
    Pop3Error::Tls {
        host: host.to_string(),
        detail: error.to_string(),
    }
}

async fn handshake_with<S: AsyncRead + AsyncWrite + Unpin>(
    host: &str,
    stream: S,
    connector: TlsConnector,
) -> Result<TlsStream<S>, Pop3Error> {
    tokio_native_tls::TlsConnector::from(connector)
        .connect(host, stream)
        .await
        .map_err(|err| failed(host, err))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use tokio::net::TcpStream;

    struct Server(Child, u16);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Server {
        fn start(directory: &std::path::Path, version: &str) -> Self {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let child = Command::new("openssl")
                .current_dir(directory)
                .args([
                    "s_server",
                    "-accept",
                    &format!("127.0.0.1:{port}"),
                    "-cert",
                    "cert.pem",
                    "-key",
                    "key.pem",
                    version,
                    "-cipher",
                    "AES256-GCM-SHA384:AES256-SHA:@SECLEVEL=0",
                    "-quiet",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("openssl is required for the TLS compatibility fixture");
            Self(child, port)
        }
        async fn tcp(&mut self) -> TcpStream {
            for _ in 0..100 {
                assert!(self.0.try_wait().unwrap().is_none(), "TLS fixture exited");
                if let Ok(stream) = TcpStream::connect(("127.0.0.1", self.1)).await {
                    return stream;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!("TLS fixture did not listen");
        }
    }

    #[tokio::test]
    async fn rsa_tls12_works_but_untrusted_hosts_and_old_protocols_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let status = Command::new("openssl")
            .current_dir(directory.path())
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-keyout",
                "key.pem",
                "-out",
                "cert.pem",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let cert = native_tls::Certificate::from_pem(
            &std::fs::read(directory.path().join("cert.pem")).unwrap(),
        )
        .unwrap();
        let connector = builder().add_root_certificate(cert).build().unwrap();
        let mut server = Server::start(directory.path(), "-tls1_2");
        let stream = handshake_with("localhost", server.tcp().await, connector.clone()).await;
        assert!(stream.is_ok(), "trusted RSA TLS 1.2 must connect");
        drop(stream);
        assert!(
            handshake_with("wrong.example", server.tcp().await, connector.clone())
                .await
                .is_err()
        );
        assert!(handshake("localhost", server.tcp().await).await.is_err());
        drop(server);
        let mut old = Server::start(directory.path(), "-tls1_1");
        assert!(
            handshake_with("localhost", old.tcp().await, connector)
                .await
                .is_err()
        );
    }
}
