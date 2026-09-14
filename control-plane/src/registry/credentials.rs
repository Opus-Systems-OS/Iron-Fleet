//! Per-agent vaults: the `credentials` block of `agents/<slug>.json` →
//! one vault per agent → one `environment_variable` credential per entry.
//! Attached only to that agent's sessions (`http::sessions`), unlike
//! `mcp_fleet::ensure_vault`'s vault, which rides on every session.
//!
//! Secrets come from the control plane's own process environment (`from_env`)
//! at sync time and go straight into the request body. Nothing here logs,
//! stores, or hashes-alone a secret: the SQLite row keeps a SHA-256 over
//! `(value, allowed_hosts)` so a rotation or a host change is detected on the
//! next sync and pushed as an in-place update.

use super::{CredentialSpec, Registry, SLUG_METADATA_KEY};
use crate::anthropic::types::{
    CredentialAuth, CredentialAuthUpdate, CredentialCreate, CredentialNetworking, CredentialUpdate,
    InjectionLocation, VaultCreate,
};
use crate::anthropic::Client;
use crate::db::{AgentCredentialRow, Db};
use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CredentialReport {
    pub vaults_created: usize,
    pub credentials_created: usize,
    pub credentials_updated: usize,
    pub credentials_unchanged: usize,
}

pub async fn sync(reg: &Registry, api: &Client, db: &Db) -> Result<CredentialReport> {
    let mut report = CredentialReport::default();

    for (slug, file) in &reg.agents {
        if file.credentials.is_empty() {
            continue;
        }

        let vault_id = match db.agent_vault(slug)? {
            Some(id) => id,
            None => {
                let vault = api
                    .create_vault(&VaultCreate {
                        display_name: format!("Iron-Fleet {slug}"),
                        metadata: BTreeMap::from([(SLUG_METADATA_KEY.to_owned(), slug.clone())]),
                    })
                    .await?;
                tracing::info!(slug, vault_id = %vault.id, "agent vault created");
                db.set_agent_vault(slug, &vault.id)?;
                report.vaults_created += 1;
                vault.id
            }
        };

        for spec in &file.credentials {
            let value = secret_from_env(slug, spec)?;
            let hash = config_hash(&value, &spec.allowed_hosts);
            let networking = CredentialNetworking::Limited {
                allowed_hosts: spec.allowed_hosts.clone(),
            };

            let credential_id = match db.agent_credential(slug, &spec.secret_name)? {
                Some(row) if row.config_sha256 == hash => {
                    report.credentials_unchanged += 1;
                    continue;
                }
                Some(row) => {
                    api.update_credential(
                        &vault_id,
                        &row.credential_id,
                        &CredentialUpdate {
                            auth: CredentialAuthUpdate::EnvironmentVariable {
                                secret_value: value,
                                networking,
                            },
                        },
                    )
                    .await?;
                    tracing::info!(slug, secret = %spec.secret_name, id = %row.credential_id, "credential rotated");
                    report.credentials_updated += 1;
                    row.credential_id
                }
                None => {
                    let created = api
                        .create_credential(
                            &vault_id,
                            &CredentialCreate {
                                display_name: format!("{slug} {}", spec.secret_name),
                                auth: CredentialAuth::EnvironmentVariable {
                                    secret_name: spec.secret_name.clone(),
                                    secret_value: value,
                                    networking,
                                    injection_location: InjectionLocation::HEADER_ONLY,
                                },
                            },
                        )
                        .await?;
                    tracing::info!(slug, secret = %spec.secret_name, id = %created.id, "credential created");
                    report.credentials_created += 1;
                    created.id
                }
            };
            db.upsert_agent_credential(
                slug,
                &spec.secret_name,
                &AgentCredentialRow {
                    credential_id,
                    config_sha256: hash,
                },
            )?;
        }
    }

    Ok(report)
}

/// Fail loud: an agent whose file promises a credential must not silently
/// start sessions without it (CLAUDE.md: "must never surprise").
fn secret_from_env(slug: &str, spec: &CredentialSpec) -> Result<String> {
    let value = std::env::var(&spec.from_env)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            Error::Config(format!(
                "agent `{slug}` credential `{}` needs {} set in the control plane's environment",
                spec.secret_name, spec.from_env
            ))
        })?;
    if value.len() > 4096 {
        return Err(Error::Config(format!(
            "{} is longer than the 4096-byte secret_value limit",
            spec.from_env
        )));
    }
    Ok(value)
}

fn config_hash(secret: &str, allowed_hosts: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    for host in allowed_hosts {
        h.update([0]);
        h.update(host.as_bytes());
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_hash_tracks_secret_and_hosts() {
        let a = config_hash("tok", &["api.github.com".into()]);
        assert_eq!(a, config_hash("tok", &["api.github.com".into()]));
        assert_ne!(a, config_hash("tok2", &["api.github.com".into()]));
        assert_ne!(a, config_hash("tok", &["github.com".into()]));
        assert_ne!(
            config_hash("a", &["b".into()]),
            config_hash("ab", &[]),
            "value and hosts are delimited, not concatenated"
        );
    }

    #[test]
    fn missing_or_blank_env_is_a_config_error_naming_the_var() {
        let spec = CredentialSpec {
            secret_name: "GH_TOKEN".into(),
            from_env: "IRON_FLEET_TEST_UNSET_TOKEN".into(),
            allowed_hosts: vec!["api.github.com".into()],
        };
        let err = secret_from_env("blueweb-client", &spec)
            .unwrap_err()
            .to_string();
        assert!(err.contains("IRON_FLEET_TEST_UNSET_TOKEN"), "{err}");
        unsafe {
            std::env::set_var("IRON_FLEET_TEST_UNSET_TOKEN", "   ");
        }
        assert!(secret_from_env("blueweb-client", &spec).is_err());
    }

    #[test]
    fn credential_auth_debug_never_prints_the_secret() {
        let auth = CredentialAuth::EnvironmentVariable {
            secret_name: "GH_TOKEN".into(),
            secret_value: "ghp_very_secret".into(),
            networking: CredentialNetworking::Limited {
                allowed_hosts: vec!["api.github.com".into()],
            },
            injection_location: InjectionLocation::HEADER_ONLY,
        };
        let s = format!("{auth:?}");
        assert!(!s.contains("very_secret"), "{s}");
        assert!(
            s.contains("GH_TOKEN") && s.contains("api.github.com"),
            "{s}"
        );

        let upd = CredentialAuthUpdate::EnvironmentVariable {
            secret_value: "ghp_rotated".into(),
            networking: CredentialNetworking::Limited {
                allowed_hosts: vec![],
            },
        };
        assert!(!format!("{upd:?}").contains("rotated"));

        // The wire shape, per the vaults doc.
        let v = serde_json::to_value(&auth).unwrap();
        assert_eq!(v["type"], "environment_variable");
        assert_eq!(v["secret_value"], "ghp_very_secret");
        assert_eq!(v["networking"]["type"], "limited");
        assert_eq!(v["injection_location"], serde_json::json!({"header": true}));
    }
}
