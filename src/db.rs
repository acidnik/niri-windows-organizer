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

/// Insert a record only if it differs from the most recent one for (app, title, position).
///
/// Using position in the lookup lets windows with the same (app, title) but different
/// scroll positions keep separate histories (e.g. two Telegram windows).
pub fn insert_if_changed(
    conn: &Connection,
    app: &str,
    title: &str,
    workspace: i64,
    position: i64,
) -> SqlResult<bool> {
    // Match by (app, title, position) so each window tracks its own history
    let latest = find_last(conn, app, title, Some(position))?;

    let changed = match latest {
        Some((last_ws, last_pos)) => last_ws != workspace || last_pos != position,
        None => true, // No prior record at this position — always insert
    };

    if changed {
        conn.execute(
            "INSERT INTO window_history (app, title, workspace, position, recorded_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![app, title, workspace, position],
        )?;
    }

    Ok(changed)
}

/// Find the most recent record for a given (app, title), optionally filtered by position.
///
/// When `pos_filter` is `Some(p)`, only records with that exact position are considered.
/// Pass `None` to get the most recent record regardless of position (used in restore).
pub fn find_last(
    conn: &Connection,
    app: &str,
    title: &str,
    pos_filter: Option<i64>,
) -> SqlResult<Option<(i64, i64)>> {
    let result = match pos_filter {
        Some(pos) => conn.query_row(
            "SELECT workspace, position FROM window_history
             WHERE app = ?1 AND title = ?2 AND position = ?3
             ORDER BY recorded_at DESC
             LIMIT 1",
            params![app, title, pos],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ),
        None => conn.query_row(
            "SELECT workspace, position FROM window_history
             WHERE app = ?1 AND title = ?2
             ORDER BY recorded_at DESC
             LIMIT 1",
            params![app, title],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ),
    };

    result.map(Some).or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

/// Find all distinct last-known (workspace, position) records for a given (app, title).
///
/// Returns the most recent record per distinct position. Used in restore when multiple
/// windows share the same (app, title) — each current window picks the record whose
/// position matches (or is closest to) its own.
pub fn find_all_positions(
    conn: &Connection,
    app: &str,
    title: &str,
) -> SqlResult<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT workspace, position FROM window_history
         WHERE app = ?1 AND title = ?2
         ORDER BY recorded_at DESC",
    )?;

    let rows = stmt.query_map(params![app, title], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;

    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for row in rows {
        let entry = row?;
        // Dedup by position: keep only the first (most recent) occurrence
        if seen.insert(entry.1) {
            results.push(entry);
        }
    }
    Ok(results)
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
