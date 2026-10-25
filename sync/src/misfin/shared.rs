//! Explicit adoption of browser identities from the de-xr catalog.
use super::*;

#[derive(Deserialize)]
struct Catalog {
    version: u32,
    identities: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    id: String,
    name: String,
    certificate: PathBuf,
    key: PathBuf,
}
#[derive(Serialize)]
pub(super) struct Available {
    id: String,
    name: String,
    address: String,
}
pub(super) fn catalog_path() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context(gettext("Cannot find the shared identity catalog"))?
        .join("de-xr/identities/index.json"))
}
fn entries(path: &Path) -> Result<Vec<Entry>> {
    let bytes = match read_private(path, 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => return Ok(vec![]),
        Err(error) => return Err(error),
    };
    let catalog: Catalog = serde_json::from_slice(&bytes)
        .context(gettext("Invalid shared identity catalog"))?;
    ensure!(catalog.version == 1 && catalog.identities.len() <= 256,
        "{}", gettext("Invalid shared identity catalog"));
    let mut ids = std::collections::BTreeSet::new();
    for entry in &catalog.identities {
        ensure!(entry.id.len() == 64
            && entry.id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            && ids.insert(&entry.id)
            && !entry.name.is_empty() && entry.name.len() <= 256
            && !entry.name.chars().any(char::is_control)
            && entry.certificate.is_absolute() && entry.key.is_absolute(),
            "{}", gettext("Invalid shared identity catalog"));
    }
    Ok(catalog.identities)
}
pub(super) fn available(path: &Path) -> Result<Vec<Available>> {
    let mut available = vec![];
    for entry in entries(path)? {
        // Generic Gemini identities are valid in the browser but cannot name a
        // Misfin mailbox. Neither listing nor filtering reads their keys.
        let Ok(certificate) = read(&entry.certificate, 65536) else { continue };
        let Ok((address, id)) = client::certificate_identity(&certificate) else { continue };
        if id == entry.id {
            available.push(Available { id, name: entry.name, address });
        }
    }
    Ok(available)
}
pub(super) fn selected(path: &Path, id: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let entry = entries(path)?.into_iter().find(|entry| entry.id == id)
        .context(gettext("Choose an available shared Misfin identity"))?;
    let certificate = read(&entry.certificate, 65536)?;
    let key = read_private(&entry.key, 65536)?;
    let (_, actual, _) = client::identity(&certificate, &key)?;
    ensure!(actual == entry.id, "{}", gettext("Shared identity certificate fingerprint changed"));
    Ok((certificate, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, String) {
        let root = tempfile::tempdir().unwrap();
        let (certificate, key) = client::create_identity("fixture@localhost", "Fixture").unwrap();
        let (_, id, _) = client::identity(&certificate, &key).unwrap();
        let cert = root.path().join("client.pem");
        let private = root.path().join("client.key");
        std::fs::write(&cert, certificate).unwrap();
        std::fs::write(&private, key).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        let catalog = root.path().join("index.json");
        std::fs::write(&catalog, serde_json::to_vec(&serde_json::json!({"version":1,"identities":[{
            "id":id,"name":"Fixture","certificate":cert,"key":private,"address":"untrusted@example.invalid"
        }]})).unwrap()).unwrap();
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o600)).unwrap();
        (root, catalog, cert, private, id)
    }
    #[test]
    fn shared_identity_adoption_revalidates_without_changing_sources() {
        let (_root, catalog, cert, private, id) = fixture();
        let before = std::fs::read(&private).unwrap();
        let list = serde_json::to_value(available(&catalog).unwrap()).unwrap();
        assert_eq!(list[0]["address"], "fixture@localhost");
        assert!(!list.to_string().contains("PRIVATE KEY"));
        let (certificate, key) = selected(&catalog, &id).unwrap();
        assert_eq!(client::identity(&certificate, &key).unwrap().1, id);
        assert_eq!(before, std::fs::read(&private).unwrap());
        let (changed, _) = client::create_identity("other@localhost", "Other").unwrap();
        std::fs::write(&cert, changed).unwrap();
        assert!(available(&catalog).unwrap().is_empty());
        assert!(selected(&catalog, &id).is_err());
    }
    #[test]
    fn shared_identity_keys_and_catalog_are_private_and_bounded() {
        let (_root, catalog, _cert, private, id) = fixture();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(selected(&catalog, &id).is_err());
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(available(&catalog).is_err());
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&catalog, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(available(&catalog).is_err());
    }
    #[test]
    fn generic_identities_are_filtered_and_relative_paths_rejected() {
        let (_root, catalog, cert, _private, id) = fixture();
        std::fs::write(&cert, b"not a Misfin certificate").unwrap();
        assert!(available(&catalog).unwrap().is_empty());
        assert!(selected(&catalog, &id).is_err());
        let mut document: serde_json::Value = serde_json::from_slice(&std::fs::read(&catalog).unwrap()).unwrap();
        document["identities"][0]["key"] = serde_json::json!("relative.key");
        std::fs::write(&catalog, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(available(&catalog).is_err());
    }
}
