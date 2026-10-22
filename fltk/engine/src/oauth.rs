// SPDX-License-Identifier: GPL-3.0-or-later
//! OAuth clients stay in the upstream account keyring, never in UI settings.
use super::*;

pub(super) fn connector(e: &Engine, r: &Value) -> Result<Connector> {
    let mut connector = e.connector.clone();
    connector.clients = Clients::current();
    if string(r, "provider") == "google" && !string(r, "client_file").is_empty() {
        connector.clients.own_google = Some(Ok(read_google_client(std::path::Path::new(string(
            r,
            "client_file",
        )))?));
    }
    Ok(connector)
}

fn read_google_client(path: &std::path::Path) -> Result<sign_in::own_client::OwnClient> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("{}", gettext("OAuth client file exceeds 1 MB"));
    }
    let value: Value =
        serde_json::from_slice(&bytes).context(gettext("Unreadable Google OAuth client JSON"))?;
    google_client(&value)
}
fn google_client(value: &Value) -> Result<sign_in::own_client::OwnClient> {
    let installed = value
        .get("installed")
        .context(gettext("Choose a Google OAuth client of type Desktop app"))?;
    let id = string(installed, "client_id").trim();
    let secret = string(installed, "client_secret").trim();
    if !id.ends_with(".apps.googleusercontent.com") || secret.is_empty() {
        bail!(
            "{}",
            gettext("The Google client file has no valid client ID and secret")
        );
    }
    // Use upstream's fixed Google endpoints; a picked file cannot redirect tokens elsewhere.
    Ok(sign_in::own_client::OwnClient::new(id, secret))
}

pub(super) fn status(e: &Engine) -> Value {
    let clients = Clients::current();
    let google = sign_in::google_sign_in(&clients);
    json!({"google":google.is_ok(),"google_error":google.err().map(|e|e.to_string()),
        "microsoft":clients.microsoft.is_some(),"demo":e.demo})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_json_is_used_without_accepting_its_endpoints() {
        let own = google_client(&json!({"installed":{"client_id":"local.apps.googleusercontent.com","client_secret":"local-secret","token_uri":"https://example.invalid/steal"}})).unwrap();
        assert_eq!(own.oauth().id(), "local.apps.googleusercontent.com");
        let url = own
            .oauth()
            .authorize_url(
                "http://127.0.0.1:1234",
                &mailrs_gmail::Pkce::generate(),
                "state",
            )
            .unwrap();
        assert!(url.starts_with("https://accounts.google.com/"));
    }
    #[test]
    fn web_clients_and_incomplete_files_are_rejected() {
        for value in [
            json!({"web":{"client_id":"web.apps.googleusercontent.com","client_secret":"x"}}),
            json!({"installed":{"client_id":"x.apps.googleusercontent.com"}}),
            json!({"installed":{"client_id":"other","client_secret":"x"}}),
        ] {
            assert!(google_client(&value).is_err());
        }
    }
}
