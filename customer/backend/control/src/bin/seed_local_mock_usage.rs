#![forbid(unsafe_code)]

#[cfg(not(all(feature = "local-demo", feature = "mariadb")))]
compile_error!("seed_local_mock_usage requires the local-demo and mariadb features");

use std::{collections::HashSet, env, fs, path::Path};

use aster_auth_core::AuthCore;
use aster_control::{ControlState, ControlStorage, ModelUsage};
use aster_machine_identity::InstallationProfile;
use aster_storage::{MariaDbConfig, MariaDbStore};
use time::{Duration, OffsetDateTime};

const MARKER: &str = "mock-ui-v1";

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = env::var(name)?.trim().to_owned();
    if value.is_empty() {
        return Err(format!("{name} is empty").into());
    }
    Ok(value)
}

fn read_secret(path: impl AsRef<Path>) -> Result<String, Box<dyn std::error::Error>> {
    let value = fs::read_to_string(path)?
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    if value.is_empty() || value.contains('\0') {
        return Err("secret file is empty or contains NUL".into());
    }
    Ok(value)
}

fn usage_for(index: usize) -> ModelUsage {
    const MODELS: [&str; 5] = [
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.6-sol",
        "gpt-image-2.5-flare",
        "claude-sonnet-4-6",
    ];
    const EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];
    let input = 2_400 + i64::try_from((index * 977) % 9_600).unwrap_or_default();
    let cache_read = if index.is_multiple_of(4) {
        input / 3
    } else {
        0
    };
    let cache_write = if index.is_multiple_of(9) {
        input / 8
    } else {
        0
    };
    let output = 380 + i64::try_from((index * 431) % 2_900).unwrap_or_default();
    let model = MODELS[index % MODELS.len()].to_owned();
    ModelUsage {
        uncached_input: input - cache_read,
        cached_input: cache_read,
        cache_write,
        output_tokens: output,
        multiplier_micros: match index % 5 {
            0 => 1_250_000,
            1 => 800_000,
            _ => 1_000_000,
        },
        protocol: if index % 6 == 4 {
            "anthropic".to_owned()
        } else {
            "openai".to_owned()
        },
        requested_model: Some(model.clone()),
        model,
        processing_tier: Some(
            if index.is_multiple_of(5) {
                "fast"
            } else {
                "standard"
            }
            .to_owned(),
        ),
        reasoning_effort: Some(EFFORTS[index % EFFORTS.len()].to_owned()),
        runner_id: "runner_local_mock".to_owned(),
    }
}

fn seed_times(anchor: OffsetDateTime) -> Vec<OffsetDateTime> {
    let mut values = Vec::with_capacity(180);
    for days_ago in (1_i64..=30).rev() {
        for slot in 0_i64..5 {
            values.push(anchor - Duration::days(days_ago) + Duration::hours(slot * 4 + 2));
        }
    }
    for hours_ago in (0_i64..24).rev() {
        values.push(anchor - Duration::hours(hours_ago));
    }
    values.sort_unstable();
    values
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_email = env::args()
        .nth(1)
        .unwrap_or_else(|| "mock.dashboard@aster.local".to_owned());
    let database_password = read_secret(required("ASTER_CONTROL_DB_PASSWORD_FILE")?)?;
    let store = MariaDbStore::open(&MariaDbConfig {
        host: required("ASTER_CONTROL_DB_HOST")?,
        port: required("ASTER_CONTROL_DB_PORT")?.parse()?,
        database: required("ASTER_CONTROL_DB_NAME")?,
        username: required("ASTER_CONTROL_DB_USER")?,
        password: database_password,
        tls: required("ASTER_CONTROL_DB_TLS")?.parse()?,
        ca_certificate: None,
        max_connections: required("ASTER_CONTROL_DB_MAX_CONNECTIONS")?.parse()?,
    })
    .await?;
    let identity = store
        .identity_by_email(&target_email)
        .await?
        .ok_or_else(|| format!("mock member {target_email} does not exist"))?;
    let api_key = store
        .api_keys_for_identity(&identity.id)
        .await?
        .into_iter()
        .find(|key| key.status == "active")
        .ok_or_else(|| format!("mock member {target_email} has no active API key"))?;
    let existing = store
        .quota_state_snapshot(&identity.id)
        .await?
        .ok_or("mock member balance is missing")?;
    let marker_prefix = format!("{MARKER}:");
    if existing
        .ledger_entries
        .iter()
        .any(|entry| entry.reference_id.starts_with(&marker_prefix))
    {
        let count = existing
            .ledger_entries
            .iter()
            .filter(|entry| entry.reference_id.starts_with(&marker_prefix))
            .count();
        println!("Mock usage already exists for {target_email}: {count} ledger entries.");
        store.close().await;
        return Ok(());
    }

    let installation_profile: InstallationProfile =
        serde_json::from_slice(&fs::read(required("ASTER_INSTALLATION_PROFILE_PATH")?)?)?;
    let installation_key = fs::read(required("ASTER_INSTALLATION_KEY_PATH")?)?;
    let auth_core = AuthCore::new(&installation_key, &installation_profile.installation_id)?;
    let state = ControlState::new("local-mock", None)
        .with_storage(ControlStorage::MariaDb(store.clone()))
        .with_auth_core(auth_core);

    let anchor = OffsetDateTime::now_utc()
        .replace_second(0)?
        .replace_nanosecond(0)?;
    let first_time = anchor - Duration::days(31);
    state
        .clone()
        .with_now(first_time)
        .grant_model_quota(
            &identity.id,
            &format!("{MARKER}:initial-grant"),
            25_000_000,
            "本地界面验收演示额度",
        )
        .await
        .map_err(|error| std::io::Error::other(format!("quota grant failed: {error:?}")))?;

    let existing_references = existing
        .ledger_entries
        .into_iter()
        .map(|entry| entry.reference_id)
        .collect::<HashSet<_>>();
    let mut inserted = 0_usize;
    for (index, created_at) in seed_times(anchor).into_iter().enumerate() {
        let request_id = format!("{MARKER}:usage:{index:03}");
        if existing_references.contains(&request_id) {
            continue;
        }
        let timed_state = state.clone().with_now(created_at);
        let reservation = timed_state
            .reserve_mock_model_quota(&identity.id, &api_key.id, &request_id, 1)
            .await
            .map_err(|error| std::io::Error::other(format!("quota reserve failed: {error:?}")))?;
        timed_state
            .settle_model_quota(&reservation, &usage_for(index))
            .await
            .map_err(|error| {
                std::io::Error::other(format!("quota settlement failed: {error:?}"))
            })?;
        inserted += 1;
    }

    let snapshot = store
        .quota_state_snapshot(&identity.id)
        .await?
        .ok_or("mock member balance disappeared")?;
    println!(
        "Seeded {inserted} mock usage entries for {target_email}; total requests={}, consumed tokens={}.",
        snapshot.balance.request_count, snapshot.balance.consumed_tokens
    );
    store.close().await;
    Ok(())
}
