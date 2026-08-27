use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OpenFlags};

use crate::{
    error::{StorageError, StorageResult},
    migration::run_migrations,
};

#[derive(Clone)]
pub struct ArchiveDb {
    conn: Arc<Mutex<Connection>>,
}

impl ArchiveDb {
    fn init_db(mut conn: Connection) -> StorageResult<Self> {
        Self::apply_pragmas(&mut conn)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        Self::init_db(Connection::open(path)?)
    }

    pub fn open_in_memory() -> StorageResult<Self> {
        Self::init_db(Connection::open_in_memory()?)
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> StorageResult<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn apply_pragmas(conn: &mut Connection) -> StorageResult<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )?;
        Ok(())
    }

    pub fn with_conn<F, R>(&self, f: F) -> StorageResult<R>
    where
        F: FnOnce(&mut Connection) -> StorageResult<R>,
    {
        let mut guard = self.conn.lock().map_err(|_| {
            StorageError::Transaction("Database connection mutex poisoned".to_string())
        })?;
        f(&mut guard)
    }
}
