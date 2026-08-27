use rusqlite::{Connection, params};
use tracing::info;
use vendetta_core::now_unix_secs;

use crate::error::StorageResult;

pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "0001_initial_schema",
        sql: include_str!("../../../migrations/0001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        description: "0002_incremental_sync",
        sql: include_str!("../../../migrations/0002_incremental_sync.sql"),
    },
    Migration {
        version: 3,
        description: "0003_media_engine",
        sql: include_str!("../../../migrations/0003_media_engine.sql"),
    },
    Migration {
        version: 4,
        description: "0004_deletion_provenance",
        sql: include_str!("../../../migrations/0004_deletion_provenance.sql"),
    },
    Migration {
        version: 5,
        description: "0005_fts5_search",
        sql: include_str!("../../../migrations/0005_fts5_search.sql"),
    },
];

pub fn run_migrations(conn: &mut Connection) -> StorageResult<usize> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL,
            description TEXT NOT NULL
        );",
    )?;

    let mut applied_count = 0;

    for migration in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            [migration.version],
            |row| row.get(0),
        )?;

        if !already_applied {
            info!(
                version = migration.version,
                description = migration.description,
                "Applying database migration"
            );

            let tx = conn.transaction()?;
            tx.execute_batch(migration.sql)?;

            let now = now_unix_secs();
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at, description) VALUES (?1, ?2, ?3)",
                params![migration.version, now, migration.description],
            )?;

            tx.commit()?;
            applied_count += 1;
        }
    }

    Ok(applied_count)
}
