//! Isolated browser fixture, excluded from production Customer feature sets.
//! The public vector key is TEST ONLY; no production trust store is changed.
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use aster_auth_core::AuthCore;
use aster_control::{ControlState, ControlStorage, RunnerTaskIssuer, install_license, web_router};
use aster_credential_vault::{CredentialContext, CredentialVault};
use aster_install_layout::InstallLayout;
use aster_license_core::{
    TrustedLicenseKeys,
    catalog::CapabilityId,
    v2::{self, BindingKind, ExpiryKind, IssuerPolicy, SourceKind},
};
use aster_license_state::LicenseStateStore;
use aster_machine_identity::{RawMachineFactors, create_profile};
use aster_storage::{DiscoveredModel, EncryptedCredentialInstance, SqlCipherStore};
use clap::{Parser, ValueEnum};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{net::TcpListener, sync::watch};
use zeroize::Zeroizing;

#[derive(Clone, Copy, ValueEnum)]
enum Capability {
    None,
    Member,
    Runner,
    Gateway,
}

#[derive(Parser)]
struct Args {
    #[arg(long)]
    ready_file: PathBuf,
    #[arg(long)]
    admin_assets: PathBuf,
    #[arg(long)]
    member_assets: PathBuf,
    #[arg(long, value_enum)]
    capability: Capability,
    #[arg(long)]
    install_root: Option<PathBuf>,
    #[arg(long)]
    seed_models: bool,
    #[arg(long)]
    expired_license: bool,
    #[arg(long)]
    free_switch: bool,
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() && receiver.changed().await.is_ok() {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    for assets in [&args.admin_assets, &args.member_assets] {
        if !assets.join("index.html").is_file() {
            return Err("build production web assets first".into());
        }
    }
    let directory = tempfile::tempdir_in(
        args.ready_file
            .parent()
            .ok_or("ready file needs a parent directory")?,
    )?;
    let vector: Value = serde_json::from_slice(include_bytes!(
        "../../../../../contracts/test-vectors/license.v2.json"
    ))?;
    let mut document: v2::Document =
        serde_json::from_value(vector["cases"][usize::from(args.free_switch)]["document"].clone())?;
    document.claims.entitlements.features = match args.capability {
        Capability::None => vec![],
        Capability::Member => vec![CapabilityId::Member],
        Capability::Runner => vec![CapabilityId::Runner],
        Capability::Gateway => vec![CapabilityId::Gateway],
    };
    let signing_key = SigningKey::from_bytes(&[42; 32]);
    let document = v2::sign(document.claims, &signing_key)?;
    let mut keys = TrustedLicenseKeys::new();
    keys.insert_scoped(
        document.claims.key_id.clone(),
        signing_key.verifying_key(),
        IssuerPolicy {
            sources: vec![SourceKind::FreeDistribution],
            bindings: vec![BindingKind::Unbound],
            expiries: vec![document.claims.validity.expiry.kind()],
            entitlement_ceiling: document.claims.entitlements.clone(),
        },
    )?;
    let profile = create_profile(
        "installation_browser_test",
        &RawMachineFactors {
            dmi_product_uuid: "browser-test-dmi".to_owned(),
            machine_id: "browser-test-os".to_owned(),
        },
    )?;
    let mut paid: v2::Document = serde_json::from_value(vector["cases"][2]["document"].clone())?;
    paid.claims.key_id = "browser-test-paid".to_owned();
    paid.claims.license_id = "test_browser_paid".to_owned();
    paid.claims.serial = "TEST-BROWSER-PAID".to_owned();
    if args.expired_license {
        paid.claims.entitlements.features = document.claims.entitlements.features.clone();
    }
    // A new license identity must advance issuance time beyond the installed free license.
    paid.claims.issued_at = "2026-09-02T00:00:00.000Z".to_owned();
    paid.claims.validity.not_before = "2026-09-02T00:00:00.000Z".to_owned();
    paid.claims.binding = v2::Binding::Installation {
        installation_id: profile.installation_id.clone(),
        machine_fingerprint_sha256: profile.machine_fingerprint_sha256.clone(),
        transfer_sequence: 0,
    };
    let paid_signer = SigningKey::from_bytes(&[44; 32]);
    let paid = v2::sign(paid.claims, &paid_signer)?;
    keys.insert_scoped(
        paid.claims.key_id.clone(),
        paid_signer.verifying_key(),
        IssuerPolicy {
            sources: vec![SourceKind::CommercialOrder],
            bindings: vec![BindingKind::Installation],
            expiries: vec![ExpiryKind::Fixed],
            entitlement_ceiling: paid.claims.entitlements.clone(),
        },
    )?;
    let bytes = serde_json::to_vec(&document)?;
    let free_file = directory.path().join("free-test.json");
    let paid_file = directory.path().join("paid-test.json");
    let without_member_file = directory.path().join("paid-without-member-test.json");
    let mut without_member = paid.claims.clone();
    without_member.entitlements.features.clear();
    std::fs::write(
        &without_member_file,
        serde_json::to_vec(&v2::sign(without_member, &paid_signer)?)?,
    )?;
    let tampered_file = directory.path().join("tampered-test.json");
    std::fs::write(&free_file, &bytes)?;
    std::fs::write(&paid_file, serde_json::to_vec(&paid)?)?;
    let mut tampered = paid.clone();
    tampered.claims.serial = "TEST-TAMPERED".to_owned();
    std::fs::write(&tampered_file, serde_json::to_vec(&tampered)?)?;
    let now = OffsetDateTime::parse("2026-09-07T00:00:00Z", &Rfc3339)?;
    let history = Arc::new(LicenseStateStore::new(
        directory.path().join("state.json"),
        &[90; 32],
    )?);
    let target = directory.path().join("license.json");
    let initial = if args.expired_license {
        serde_json::to_vec(&paid)?
    } else {
        bytes
    };
    let license = install_license(
        &initial,
        &target,
        &keys,
        &profile,
        &history,
        env!("CARGO_PKG_VERSION"),
        now,
    )
    .map_err(|error| format!("install fixture license: {error:?}"))?;
    let mut store = SqlCipherStore::initialize(&directory.path().join("customer.db"), &[91; 32])?;
    let seeded_models = args.seed_models || args.install_root.is_some();
    if seeded_models {
        // Model discovery predates this member-only license. Seed actual stored
        // resources without granting gateway/runner or mocking member responses.
        store.insert_upstream_account_unchecked(
            "account_browser_docs",
            "openai",
            "browser-docs-subject",
            "docs@example.test",
            "2026-09-07T00:00:00.000Z",
        )?;
        let vault = CredentialVault::new(&[95; 32], "installation_browser_test")?;
        let material = vault.encrypt(
            &CredentialContext {
                credential_id: "credential_browser_docs",
                account_id: "account_browser_docs",
                revision: 0,
            },
            br#"{"access_token":"browser-docs-test-secret"}"#,
        )?;
        store.insert_credential_instance(&EncryptedCredentialInstance {
            id: "credential_browser_docs".to_owned(),
            account_id: "account_browser_docs".to_owned(),
            credential_identity_hmac: vault
                .credential_identity_hmac("openai", b"browser-docs-test-secret")?,
            encrypted_payload: material.encrypted_payload,
            payload_nonce: material.payload_nonce,
            wrapped_data_key: material.wrapped_data_key,
            wrap_nonce: material.wrap_nonce,
            credential_revision: 0,
            expires_at: "2026-09-08T00:00:00.000Z".to_owned(),
            status: "active".to_owned(),
            last_refreshed_at: None,
            created_at: "2026-09-07T00:00:00.000Z".to_owned(),
            updated_at: "2026-09-07T00:00:00.000Z".to_owned(),
        })?;
        store.replace_account_models(
            "account_browser_docs",
            &[DiscoveredModel {
                id: "model_browser_docs".to_owned(),
                public_name: "gpt-5.6-sol".to_owned(),
                display_name: "Browser Docs Model".to_owned(),
                upstream_name: "gpt-5.6-sol".to_owned(),
            }],
            "2026-09-07T00:00:00.000Z",
        )?;
    }
    let store = Arc::new(Mutex::new(store));
    let mut state = ControlState::new(env!("CARGO_PKG_VERSION"), Some(license))
        .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
        .with_license_state(history)
        .with_license_installer(target, keys, profile)
        .with_auth_core(AuthCore::new(&[90; 32], "installation_browser_test")?)
        .with_task_issuer(RunnerTaskIssuer::new(
            "browser-test-runner",
            SigningKey::from_bytes(&[43; 32]),
        ))
        .with_secure_session_cookies(false)
        .with_now(now)
        .with_settlement_outbox(&InstallLayout::new(directory.path().to_path_buf())?)
        .map_err(|error| format!("configure fixture settlement outbox: {error:?}"))?;
    if seeded_models {
        state = state.with_credential_vault(CredentialVault::new(
            &[95; 32],
            "installation_browser_test",
        )?);
    }
    if let Some(root) = args.install_root {
        state = state.with_maintenance_layout(InstallLayout::new(root)?);
    } else if args.free_switch {
        let layout = InstallLayout::new(directory.path().join("installation"))?;
        let bundled = layout.current().join("licenses/free-license.json");
        std::fs::create_dir_all(bundled.parent().ok_or("bundled free path needs parent")?)?;
        std::fs::copy(&free_file, bundled)?;
        state = state.with_maintenance_layout(layout);
    }
    state
        .initialize_owner_identity(
            "owner@example.test",
            "Browser Owner",
            Zeroizing::new(b"owner-password-strong".to_vec()),
        )
        .await
        .map_err(|error| format!("initialize fixture owner: {error:?}"))?;
    if args.expired_license {
        state
            .reset_admin_password(
                "owner@example.test",
                Zeroizing::new(b"owner-password-strong".to_vec()),
            )
            .await
            .map_err(|error| format!("prepare expiry owner: {error:?}"))?;
        let owner = store
            .lock()
            .map_err(|_| "fixture database lock")?
            .identity_by_email("owner@example.test")?
            .ok_or("missing fixture owner")?;
        state
            .create_member_identity(
                &owner,
                "retained@example.test",
                "Retained member",
                Zeroizing::new(b"member-password-strong".to_vec()),
            )
            .await
            .map_err(|error| format!("prepare expiry member: {error:?}"))?;
        state = state.with_now(OffsetDateTime::parse("2027-09-02T00:00:00Z", &Rfc3339)?);
    }
    let admin = TcpListener::bind("127.0.0.1:0").await?;
    let member = TcpListener::bind("127.0.0.1:0").await?;
    let admin_url = format!("http://{}", admin.local_addr()?);
    let member_url = format!("http://{}", member.local_addr()?);
    state
        .initialize_runtime_configuration(&admin_url)
        .await
        .map_err(|error| format!("initialize fixture settings: {error:?}"))?;
    let ready = json!({"admin_url":admin_url,"member_url":member_url,"features":document.claims.entitlements.features,
        "runner_protocol":aster_runner_protocol::RUNNER_PROTOCOL_VERSION,"product_version":env!("CARGO_PKG_VERSION"),
        "license_files":{"free":free_file,"paid":paid_file,"tampered":tampered_file,"without_member":without_member_file},
        "paid_license_id":paid.claims.license_id});
    let (shutdown, receiver) = watch::channel(false);
    tokio::task::spawn_blocking(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let _ = shutdown.send(true);
    });
    // The parent owns this path and reads only after the atomic rename.
    let staging = args.ready_file.with_extension("tmp");
    std::fs::write(&staging, serde_json::to_vec(&ready)?)?;
    std::fs::rename(staging, &args.ready_file)?;
    tokio::try_join!(
        axum::serve(admin, web_router(state.clone(), &args.admin_assets))
            .with_graceful_shutdown(wait_for_shutdown(receiver.clone())),
        axum::serve(member, web_router(state.clone(), &args.member_assets))
            .with_graceful_shutdown(wait_for_shutdown(receiver)),
    )?;
    state.drain_licensed_mutations().await;
    Ok(())
}
