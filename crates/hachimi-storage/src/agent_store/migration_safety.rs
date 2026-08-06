use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    ptr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{SqlitePool, migrate::Migrator};

use super::AgentStoreError;

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_RETRY: Duration = Duration::from_millis(100);
const RETAINED_BACKUPS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MigrationBackupManifest {
    from_migration: i64,
    to_migration: i64,
    protocol_version: u32,
    created_at_ms: i64,
    database_sha256: String,
    database_file: String,
}

pub(super) async fn run_migrations(
    pool: &SqlitePool,
    database_path: Option<&Path>,
    migrator: &Migrator,
) -> Result<(), AgentStoreError> {
    let target = migrator
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);
    let mut applied = applied_version(pool).await?;
    if applied >= target {
        return Ok(());
    }

    let _lock = if let Some(path) = database_path {
        Some(acquire_migration_lock(path).await?)
    } else {
        None
    };

    // Another process may have completed the upgrade while this process waited for the lock.
    applied = applied_version(pool).await?;
    if applied >= target {
        return Ok(());
    }

    if let Some(path) = database_path
        && applied > 0
        && path.is_file()
    {
        create_online_backup(pool, path, applied, target).await?;
    }

    migrator.run(pool).await?;
    Ok(())
}

async fn applied_version(pool: &SqlitePool) -> Result<i64, AgentStoreError> {
    let result = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(pool)
    .await;
    match result {
        Ok(version) => Ok(version.unwrap_or(0)),
        Err(sqlx::Error::Database(error))
            if error.message().contains("no such table: _sqlx_migrations") =>
        {
            Ok(0)
        }
        Err(error) => Err(error.into()),
    }
}

async fn acquire_migration_lock(database_path: &Path) -> Result<File, AgentStoreError> {
    acquire_migration_lock_with_timeout(database_path, LOCK_TIMEOUT).await
}

async fn acquire_migration_lock_with_timeout(
    database_path: &Path,
    timeout: Duration,
) -> Result<File, AgentStoreError> {
    let lock_path = PathBuf::from(format!("{}.migrate.lock", database_path.display()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    let started = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error) if is_lock_contended(&error) => {
                if started.elapsed() >= timeout {
                    return Err(AgentStoreError::DatabaseMigrationBusy);
                }
                tokio::time::sleep(LOCK_RETRY).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || cfg!(windows)
            && (error.kind() == std::io::ErrorKind::PermissionDenied
                || matches!(error.raw_os_error(), Some(32 | 33)))
}

async fn create_online_backup(
    pool: &SqlitePool,
    database_path: &Path,
    from_migration: i64,
    to_migration: i64,
) -> Result<PathBuf, AgentStoreError> {
    let backup_dir = PathBuf::from(format!("{}.backups", database_path.display()));
    fs::create_dir_all(&backup_dir)?;
    let created_at_ms = now_ms();
    let stem = database_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("hachimi");
    let backup_path = backup_dir.join(format!(
        "{stem}-pre-v{from_migration}-to-v{to_migration}-{created_at_ms}.sqlite3"
    ));

    let destination = CString::new(backup_path.to_string_lossy().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let main = c"main";
    let mut connection = pool.acquire().await?;
    let mut source = connection.lock_handle().await?;
    let mut destination_handle = ptr::null_mut();

    // SAFETY: destination_handle is owned here; source remains locked throughout the backup;
    // every SQLite return code is checked before the handles are released.
    unsafe {
        let open_result = libsqlite3_sys::sqlite3_open_v2(
            destination.as_ptr(),
            &mut destination_handle,
            libsqlite3_sys::SQLITE_OPEN_READWRITE | libsqlite3_sys::SQLITE_OPEN_CREATE,
            ptr::null(),
        );
        if open_result != libsqlite3_sys::SQLITE_OK {
            if !destination_handle.is_null() {
                libsqlite3_sys::sqlite3_close(destination_handle);
            }
            return Err(sqlite_backup_error("open", open_result).into());
        }
        let backup = libsqlite3_sys::sqlite3_backup_init(
            destination_handle,
            main.as_ptr(),
            source.as_raw_handle().as_ptr(),
            main.as_ptr(),
        );
        if backup.is_null() {
            let code = libsqlite3_sys::sqlite3_errcode(destination_handle);
            libsqlite3_sys::sqlite3_close(destination_handle);
            return Err(sqlite_backup_error("init", code).into());
        }
        let step_result = libsqlite3_sys::sqlite3_backup_step(backup, -1);
        let finish_result = libsqlite3_sys::sqlite3_backup_finish(backup);
        let close_result = libsqlite3_sys::sqlite3_close(destination_handle);
        if step_result != libsqlite3_sys::SQLITE_DONE {
            return Err(sqlite_backup_error("step", step_result).into());
        }
        if finish_result != libsqlite3_sys::SQLITE_OK {
            return Err(sqlite_backup_error("finish", finish_result).into());
        }
        if close_result != libsqlite3_sys::SQLITE_OK {
            return Err(sqlite_backup_error("close", close_result).into());
        }
    }
    drop(source);
    drop(connection);

    let manifest = MigrationBackupManifest {
        from_migration,
        to_migration,
        protocol_version: hachimi_protocol::CONTROL_PROTOCOL_VERSION,
        created_at_ms,
        database_sha256: sha256_file(&backup_path)?,
        database_file: backup_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
    };
    fs::write(
        backup_path.with_extension("sqlite3.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    prune_backups(&backup_dir)?;
    Ok(backup_path)
}

fn prune_backups(directory: &Path) -> Result<(), AgentStoreError> {
    let mut backups = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sqlite3"))
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(RETAINED_BACKUPS);
    for backup in backups.into_iter().take(remove_count) {
        remove_if_present(&backup)?;
        remove_if_present(&backup.with_extension("sqlite3.json"))?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), AgentStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sha256_file(path: &Path) -> Result<String, AgentStoreError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn sqlite_backup_error(stage: &str, code: i32) -> std::io::Error {
    std::io::Error::other(format!(
        "sqlite online backup {stage} failed with code {code}"
    ))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, time::Duration};

    use fs2::FileExt;
    use sqlx::{
        migrate::Migrator,
        sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    };

    use super::*;
    use crate::AgentStore;

    async fn pool(path: &Path) -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
                    .expect("sqlite options")
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal),
            )
            .await
            .expect("sqlite pool")
    }

    fn write_migration(directory: &Path, version: u32, name: &str, sql: &str) {
        fs::write(directory.join(format!("{version:04}_{name}.sql")), sql)
            .expect("migration fixture");
    }

    #[tokio::test]
    async fn upgrades_v18_to_latest_with_online_backup_manifest() {
        let fixture = tempfile::tempdir().expect("fixture");
        let old_migrations = fixture.path().join("migrations-v18");
        fs::create_dir(&old_migrations).expect("migration directory");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for version in 1..=18 {
            let prefix = format!("{version:04}_");
            let migration = fs::read_dir(&source)
                .expect("source migrations")
                .filter_map(Result::ok)
                .find(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
                .expect("versioned migration");
            fs::copy(migration.path(), old_migrations.join(migration.file_name()))
                .expect("copy migration");
        }
        let database = fixture.path().join("agent.sqlite3");
        let old_pool = pool(&database).await;
        Migrator::new(old_migrations.as_path())
            .await
            .expect("v18 migrator")
            .run(&old_pool)
            .await
            .expect("v18 database");
        old_pool.close().await;

        let store = AgentStore::connect(&database).await.expect("upgrade");
        let version: i64 =
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(store.pool())
                .await
                .expect("migration version");
        assert_eq!(version, 41);

        let backup_directory = PathBuf::from(format!("{}.backups", database.display()));
        let manifest_path = fs::read_dir(&backup_directory)
            .expect("backup directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .expect("backup manifest");
        let manifest: MigrationBackupManifest =
            serde_json::from_slice(&fs::read(manifest_path).expect("manifest bytes"))
                .expect("manifest");
        assert_eq!((manifest.from_migration, manifest.to_migration), (18, 41));
        assert_eq!(manifest.protocol_version, 31);
        let backup = backup_directory.join(&manifest.database_file);
        assert_eq!(
            manifest.database_sha256,
            sha256_file(&backup).expect("hash")
        );
    }

    #[tokio::test]
    async fn failed_migration_rolls_back_and_preserves_backup() {
        let fixture = tempfile::tempdir().expect("fixture");
        let database = fixture.path().join("failure.sqlite3");
        let initial_dir = fixture.path().join("initial");
        fs::create_dir(&initial_dir).expect("initial dir");
        write_migration(
            &initial_dir,
            1,
            "initial",
            "CREATE TABLE stable(id INTEGER PRIMARY KEY);",
        );
        let database_pool = pool(&database).await;
        Migrator::new(initial_dir.as_path())
            .await
            .expect("initial migrator")
            .run(&database_pool)
            .await
            .expect("initial migration");

        let failing_dir = fixture.path().join("failing");
        fs::create_dir(&failing_dir).expect("failing dir");
        write_migration(
            &failing_dir,
            1,
            "initial",
            "CREATE TABLE stable(id INTEGER PRIMARY KEY);",
        );
        write_migration(
            &failing_dir,
            2,
            "broken",
            "CREATE TABLE doomed(id INTEGER PRIMARY KEY); THIS IS NOT SQL;",
        );
        let migrator = Migrator::new(failing_dir.as_path())
            .await
            .expect("failing migrator");
        assert!(
            run_migrations(&database_pool, Some(&database), &migrator)
                .await
                .is_err()
        );
        let applied: i64 =
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(&database_pool)
                .await
                .expect("applied version");
        assert_eq!(applied, 1);
        let doomed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'doomed'",
        )
        .fetch_one(&database_pool)
        .await
        .expect("doomed lookup");
        assert_eq!(doomed, 0);
        let backup_directory = PathBuf::from(format!("{}.backups", database.display()));
        assert!(
            fs::read_dir(backup_directory)
                .expect("backup directory")
                .filter_map(Result::ok)
                .any(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("sqlite3")
                )
        );
    }

    #[tokio::test]
    async fn migration_lock_is_exclusive_and_reports_busy() {
        let fixture = tempfile::tempdir().expect("fixture");
        let database = fixture.path().join("locked.sqlite3");
        let lock_path = PathBuf::from(format!("{}.migrate.lock", database.display()));
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .expect("owner lock file");
        owner.lock_exclusive().expect("owner lock");
        let error = acquire_migration_lock_with_timeout(&database, Duration::from_millis(25))
            .await
            .expect_err("second migrator must fail closed");
        assert!(matches!(error, AgentStoreError::DatabaseMigrationBusy));
        owner.unlock().expect("unlock");
    }

    #[tokio::test]
    async fn backup_retention_keeps_the_latest_three_pairs() {
        let fixture = tempfile::tempdir().expect("fixture");
        let database = fixture.path().join("retention.sqlite3");
        let database_pool = pool(&database).await;
        sqlx::query("CREATE TABLE stable(id INTEGER PRIMARY KEY)")
            .execute(&database_pool)
            .await
            .expect("schema");
        for version in 1..=4 {
            create_online_backup(&database_pool, &database, version, version + 1)
                .await
                .expect("backup");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let backup_directory = PathBuf::from(format!("{}.backups", database.display()));
        let entries = fs::read_dir(backup_directory)
            .expect("backup directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        assert_eq!(
            entries
                .iter()
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sqlite3"))
                .count(),
            3
        );
        assert_eq!(
            entries
                .iter()
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
                .count(),
            3
        );
    }
}
