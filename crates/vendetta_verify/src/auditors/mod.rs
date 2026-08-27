use rusqlite::Connection;

pub mod db;
pub mod export;
pub mod fs;
pub mod reply_graph;

pub struct DatabaseAuditContext<'a> {
    pub conn: &'a Connection,
    pub is_fast_mode: bool,
}
