// SPDX-License-Identifier: GPL-3.0-or-later
//! Account forms use the same checks and keyring stores as the upstream app.
use super::*;
pub async fn request(e: &Engine, r: &Value) -> Result<Value> {
    if e.demo {
        bail!("{}", gettext("Demo mode cannot change real accounts"));
    }
    match string(r, "op") {
        "add-caldav" => {
            let new = sign_in::NewCalDav {
                url: string(r, "url").into(),
                user_name: string(r, "username").into(),
                password: string(r, "password").into(),
            };
            let a = sign_in::caldav_signed_in(
                &mailrs_sync::services::finding::RealProbe,
                &e.db,
                e.connector.secrets.passwords.clone(),
                new,
                now_millis(),
            )
            .await?;
            e.connect(&a).await?;
            Ok(json!({"account":a}))
        }
        "add-jmap" => {
            let user = string(r, "username");
            let secret = string(r, "password");
            let bearer = r["bearer"].as_bool().unwrap_or(true);
            let auth = if bearer {
                mailrs_jmap::Auth::Bearer(secret.into())
            } else {
                mailrs_jmap::Auth::Basic {
                    user: user.into(),
                    password: secret.into(),
                }
            };
            let client = mailrs_jmap::JmapClient::new(string(r, "url"), auth)?;
            let checked = mailrs_jmap::check(&client).await?;
            let signed_as = sign_in::SignedAs::of(&checked);
            let saved = mailrs_store::jmap::JmapSaved {
                session_url: string(r, "url").into(),
                auth: if bearer {
                    mailrs_store::jmap::JmapAuth::Bearer
                } else {
                    mailrs_store::jmap::JmapAuth::Basic
                },
                user_name: Some(user.into()),
                labels: false,
                capabilities: checked.session.capabilities.keys().cloned().collect(),
                oauth: None,
                owner: Some(signed_as.owner.clone()),
            };
            let new = sign_in::NewJmap {
                address: string(r, "address").into(),
                provider_name: "JMAP".into(),
                saved,
                secret: secret.into(),
                dav_password: None,
                signed_as,
            };
            let a = sign_in::jmap_signed_in(&e.db, &e.connector.secrets, new, now_millis()).await?;
            e.connect(&a).await?;
            Ok(json!({"account":a}))
        }
        "add-pop3" => {
            let incoming: mailrs_discover::Server = serde_json::from_value(r["imap"].clone())?;
            let outgoing: mailrs_discover::Server = serde_json::from_value(r["smtp"].clone())?;
            let address = string(r, "address");
            let user = r["username"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(address);
            let password = string(r, "password");
            let incoming_user = sign_in::check_pop3(&incoming, user, password).await?;
            let out_user = r["smtp_username"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(user);
            let out_password = r["smtp_password"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(password);
            let smtp_user =
                mailrs_imap::check_smtp(&outgoing, &incoming_user, out_user, out_password).await?;
            let new = sign_in::NewPop3 {
                address: address.into(),
                provider_name: "POP3".into(),
                servers: mailrs_sync::pop3_servers_for(
                    &incoming,
                    &incoming_user,
                    &outgoing,
                    &smtp_user,
                ),
                remove: mailrs_domain::RemoveSetting::Never,
                password: password.into(),
                smtp_password: Some(out_password.into()),
            };
            let a = sign_in::pop3_signed_in(
                &e.db,
                e.connector.secrets.passwords.clone(),
                new,
                now_millis(),
            )
            .await?;
            e.connect(&a).await?;
            Ok(json!({"account":a}))
        }
        "remove-account" => {
            let id = account(r);
            let a = e
                .accounts()
                .await?
                .into_iter()
                .find(|a| a.id == id)
                .context(gettext("Account not found"))?;
            e.sync.stop_account(id);
            e.connector.forget(&a).await?;
            e.db.write(move |c| accounts::delete_account(c, id)).await?;
            Ok(json!({}))
        }
        _ => bail!("{}", gettext("Unknown account request")),
    }
}
