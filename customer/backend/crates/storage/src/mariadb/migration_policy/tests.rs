use super::{MariaDbMigrationPolicy as Policy, StorageError};
use crate::mariadb::{
    EmbeddedMigration, MARIADB_MIGRATIONS, MariaDbConfig, MariaDbStore, MigrationCompatibility,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[tokio::test(start_paused = true)]
async fn client_deadlines_drop_owned_operations_but_do_not_limit_maintenance() {
    struct Owned(Arc<AtomicBool>);
    impl Drop for Owned {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    for (policy, seconds) in [(Policy::Online, 30), (Policy::Existing, 10)] {
        let dropped = Arc::new(AtomicBool::new(false));
        let owner = Owned(Arc::clone(&dropped));
        let task = tokio::spawn(policy.bounded(async move {
            let _owner = owner;
            std::future::pending::<Result<(), StorageError>>().await
        }));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(seconds - 1)).await;
        assert!(!task.is_finished());
        assert!(!dropped.load(Ordering::SeqCst));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(
            task.await.unwrap(),
            Err(StorageError::MigrationTimeout)
        ));
        assert!(dropped.load(Ordering::SeqCst));
    }
    let maintenance = tokio::spawn(Policy::Maintenance.bounded(async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(())
    }));
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(!maintenance.is_finished());
    tokio::time::advance(Duration::from_secs(29)).await;
    maintenance.await.unwrap().unwrap();
}

pub(in crate::mariadb) async fn verify_online_policy(config: &MariaDbConfig) {
    let mut config = config.clone();
    config.database = "aster_online_migration_fixture".into();
    let store = MariaDbStore::connect(&config).await.unwrap();
    store
        .apply_migrations(&MARIADB_MIGRATIONS[..1])
        .await
        .unwrap();
    store
        .verify_installation("online_installation", &[12; 32], true)
        .await
        .unwrap();
    assert!(matches!(
        MariaDbStore::open_installed_with_policy(
            &config,
            "wrong_installation",
            &[12; 32],
            false,
            Policy::Online,
        )
        .await,
        Err(StorageError::InstallationMismatch)
    ));
    assert!(matches!(
        MariaDbStore::open_installed_with_policy(
            &config,
            "online_installation",
            &[12; 32],
            true,
            Policy::Online,
        )
        .await,
        Err(StorageError::MigrationIntegrity)
    ));
    assert!(matches!(
        MariaDbStore::open_installed_with_policy(
            &config,
            "online_installation",
            &[12; 32],
            false,
            Policy::Existing,
        )
        .await,
        Err(StorageError::UnsupportedSchema { version: 2 })
    ));
    assert_eq!(store.schema_version().await.unwrap(), 1);

    // A live old transaction owns table metadata. Online DDL must not queue
    // behind it, close it, or silently fall back to maintenance/copying SQL.
    let mut previous = store.pool().begin().await.unwrap();
    let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ledger_entries")
        .fetch_one(&mut *previous)
        .await
        .unwrap();
    let started = Instant::now();
    let attempted = MariaDbStore::open_installed_with_policy(
        &config,
        "online_installation",
        &[12; 32],
        false,
        Policy::Online,
    )
    .await;
    match attempted {
        Err(StorageError::MariaDb(sqlx::Error::Database(error))) => {
            assert_eq!(
                error
                    .downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    .number(),
                1205
            );
        }
        Err(other) => panic!("expected a metadata lock rejection: {other:?}"),
        Ok(_) => panic!("online DDL waited through or bypassed the old transaction"),
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(store.schema_version().await.unwrap(), 1);
    let column_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='ledger_entries' AND COLUMN_NAME='requested_model'")
        .fetch_one(store.pool()).await.unwrap();
    assert_eq!(column_count, 0);
    let old_query: i64 = sqlx::query_scalar("SELECT 42")
        .fetch_one(&mut *previous)
        .await
        .unwrap();
    assert_eq!(old_query, 42, "the old transaction must remain usable");
    previous.rollback().await.unwrap();

    let upgraded = MariaDbStore::open_installed_with_policy(
        &config,
        "online_installation",
        &[12; 32],
        false,
        Policy::Online,
    )
    .await
    .unwrap();
    assert_eq!(
        upgraded.schema_version().await.unwrap(),
        crate::CURRENT_SCHEMA_VERSION
    );
    upgraded.verify_read_write().await.unwrap();
    upgraded.close().await;
    let candidate = MariaDbStore::open_installed_with_policy(
        &config,
        "online_installation",
        &[12; 32],
        false,
        Policy::Existing,
    )
    .await
    .unwrap();
    candidate.close().await;

    let mut plan = MARIADB_MIGRATIONS.to_vec();
    plan.push(EmbeddedMigration {
        version: crate::CURRENT_SCHEMA_VERSION + 1,
        name: "online-first-fixture",
        sql: "CREATE TABLE online_should_not_run(id INT)",
        online_sql: Some("CREATE TABLE online_should_not_run(id INT)"),
        recovery_query: Some("SELECT 0"),
        compatibility: MigrationCompatibility::RollingUpgradeSafe,
    });
    plan.push(EmbeddedMigration {
        version: crate::CURRENT_SCHEMA_VERSION + 2,
        name: "unclassified-online-fixture",
        sql: "SELECT 1",
        online_sql: None,
        recovery_query: Some("SELECT 0"),
        compatibility: MigrationCompatibility::RollingUpgradeSafe,
    });
    assert!(matches!(
        store
            .apply_migrations_with_policy(&plan, Policy::Online)
            .await,
        Err(StorageError::MigrationIntegrity)
    ));
    let partial: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='online_should_not_run'")
        .fetch_one(store.pool()).await.unwrap();
    assert_eq!(
        partial, 0,
        "classify every pending migration before the first SQL"
    );
    assert_eq!(
        store.schema_version().await.unwrap(),
        crate::CURRENT_SCHEMA_VERSION
    );

    let mut owner = store.pool().acquire().await.unwrap().detach();
    let acquired: Option<i64> =
        sqlx::query_scalar("SELECT GET_LOCK('aster_team_schema_migrations', 0)")
            .fetch_one(&mut owner)
            .await
            .unwrap();
    assert_eq!(acquired, Some(1));
    assert!(matches!(
        MariaDbStore::open_installed_with_policy(
            &config,
            "online_installation",
            &[12; 32],
            false,
            Policy::Online,
        )
        .await,
        Err(StorageError::MigrationIntegrity)
    ));
    let released: Option<i64> =
        sqlx::query_scalar("SELECT RELEASE_LOCK('aster_team_schema_migrations')")
            .fetch_one(&mut owner)
            .await
            .unwrap();
    assert_eq!(
        released,
        Some(1),
        "online rejection must not steal another session's lock"
    );
    sqlx::Connection::close(owner).await.unwrap();
    store.close().await;
}
