use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqlResult};

#[allow(dead_code)]
pub struct WindowEntry {
    pub app: String,
    pub title: String,
    pub workspace: i64,
    pub position: i64,
    pub recorded_at: DateTime<Utc>,
}

pub fn open(db_path: &str) -> SqlResult<Connection> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS window_history (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            app         TEXT NOT NULL,
            title       TEXT NOT NULL,
            workspace   INTEGER NOT NULL,
            position    INTEGER NOT NULL DEFAULT 0,
            recorded_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_window_history_app_title
            ON window_history(app, title, recorded_at DESC);

        CREATE INDEX IF NOT EXISTS idx_window_history_recorded_at
            ON window_history(recorded_at);",
    )?;
    Ok(conn)
}

/// Insert a record only if it differs from the most recent one for (app, title).
pub fn insert_if_changed(
    conn: &Connection,
    app: &str,
    title: &str,
    workspace: i64,
    position: i64,
) -> SqlResult<bool> {
    let changed: bool = conn
        .query_row(
            "SELECT 1 FROM window_history
             WHERE app = ?1 AND title = ?2
             ORDER BY recorded_at DESC
             LIMIT 1
             HAVING workspace != ?3 OR position != ?4",
            params![app, title, workspace, position],
            |_row| Ok(true),
        )
        .unwrap_or(true);

    if changed {
        conn.execute(
            "INSERT INTO window_history (app, title, workspace, position, recorded_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![app, title, workspace, position],
        )?;
    }

    Ok(changed)
}

/// Find the most recent record for a given (app, title).
/// Returns (workspace, position).
pub fn find_last(conn: &Connection, app: &str, title: &str) -> SqlResult<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT workspace, position FROM window_history
         WHERE app = ?1 AND title = ?2
         ORDER BY recorded_at DESC
         LIMIT 1",
        params![app, title],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map(Some)
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

/// Delete records older than 1 year.
pub fn prune_old(conn: &Connection) -> SqlResult<usize> {
    let count = conn.execute(
        "DELETE FROM window_history
         WHERE recorded_at < datetime('now', '-1 year')",
        [],
    )?;
    Ok(count)
}
