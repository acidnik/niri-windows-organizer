mod db;
mod niri;

use clap::{Parser, Subcommand};
use std::collections::{HashMap, HashSet};
use std::process;
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "niri-windows-organizer", about = "Save and restore window positions on niri")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Periodically save window positions to SQLite
    Watch {
        /// Interval in seconds between saves
        #[arg(short, long, default_value = "10")]
        interval: u64,

        /// Path to the SQLite database
        #[arg(short, long, default_value = "~/.local/share/niri-windows-organizer/history.db")]
        db: String,
    },
    /// Restore windows to their last known workspaces
    Restore {
        /// Path to the SQLite database
        #[arg(short, long, default_value = "~/.local/share/niri-windows-organizer/history.db")]
        db: String,
    },
}

fn run_watch(db_path: &str, interval_secs: u64) {
    let db_path = shellexpand::tilde(db_path).into_owned();

    // Ensure directory exists
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let conn = db::open(&db_path).unwrap_or_else(|e| {
        eprintln!("Failed to open database: {e}");
        process::exit(1);
    });

    // Prune old records at startup
    match db::prune_old(&conn) {
        Ok(n) if n > 0 => eprintln!("Pruned {n} old record(s)"),
        _ => {}
    }

    let interval = Duration::from_secs(interval_secs);
    eprintln!("Watching every {interval_secs}s, db={db_path}");

    loop {
        let windows = match niri::get_windows() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Error getting windows: {e}");
                thread::sleep(interval);
                continue;
            }
        };

        // Sort so dedup below is deterministic — always keeps lowest ID per (app, title)
        let mut sorted_windows = windows.clone();
        sorted_windows.sort_by_key(|w| (w.app_id.clone(), w.title.clone(), w.id));

        let mut changes: Vec<String> = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        for w in &sorted_windows {
            let key = format!("{}|{}", w.app_id, w.title);
            if !seen_keys.insert(key) {
                continue;
            }
            let pos = w.layout.pos_in_scrolling_layout[0];
            let changed = match db::find_last(&conn, &w.app_id, &w.title, Some(pos)) {
                Ok(Some((old_ws, _))) => {
                    // Position matched, only compare workspace
                    if old_ws != w.workspace_id {
                        Some(old_ws)
                    } else {
                        None
                    }
                }
                Ok(None) => {
                    // No record at this position — new window or position changed
                    Some(w.workspace_id) // triggers insert; detected below
                }
                Err(e) => {
                    eprintln!("DB query error: {e}");
                    None
                }
            };

            if let Some(old_ws) = changed {
                match db::insert_if_changed(&conn, &w.app_id, &w.title, w.workspace_id, pos) {
                    Ok(true) => {
                        if old_ws == w.workspace_id {
                            changes.push(
                                format!("  Window {:>4} \"{}\" [{}]: new (ws={}, pos={})",
                                    w.id, w.title, w.app_id, w.workspace_id, pos),
                            );
                        } else {
                            changes.push(
                                format!("  Window {:>4} \"{}\" [{}]: ws: {}→{}",
                                    w.id, w.title, w.app_id, old_ws, w.workspace_id),
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("DB insert error: {e}"),
                }
            }
        }

        if !changes.is_empty() {
            println!("Window change(s):");
            for line in &changes {
                println!("{line}");
            }
        }

        thread::sleep(interval);
    }
}

fn run_restore(db_path: &str) {
    let db_path = shellexpand::tilde(db_path).into_owned();
    let conn = db::open(&db_path).unwrap_or_else(|e| {
        eprintln!("Failed to open database: {e}");
        process::exit(1);
    });

    // 1. Get all current windows
    let windows = match niri::get_windows() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Error getting windows: {e}");
            process::exit(1);
        }
    };

    // 2. For each window, find its last known (workspace, position) from DB.
    //    Prefer records whose position matches the current position (for windows with
    //    identical app_id+title, e.g. two Telegram windows). Fall back to closest
    //    position, or the most recent record if nothing else matches.
    //    Each entry: (window_id, app_id, title, target_workspace, target_position).
    let mut to_restore: Vec<(u64, String, String, i64, i64)> = Vec::new();

    // Collect DB records per (app, title) to avoid re-querying
    let mut record_cache: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    // Deduplicate by (app, title) — multiple identical windows cause ping-pong restore
    let mut seen_keys: HashSet<String> = HashSet::new();

    for w in &windows {
        let cur_ws = w.workspace_id;
        let cur_pos = w.layout.pos_in_scrolling_layout[0];
        let key = format!("{}|{}", w.app_id, w.title);
        if !seen_keys.insert(key.clone()) {
            eprintln!(
                "  Window {:>4} \"{}\" [{}]: duplicate of another window — skipping",
                w.id, w.title, w.app_id
            );
            continue;
        }

        let records = record_cache.entry(key).or_insert_with(|| {
            db::find_all_positions(&conn, &w.app_id, &w.title).unwrap_or_default()
        });

        if records.is_empty() {
            eprintln!(
                "  Window {:>4} \"{}\" [{}]: no DB records found — skipping",
                w.id, w.title, w.app_id
            );
            continue;
        }

        // Find best matching record: exact position match, closest position, or most recent
        let best = records
            .iter()
            .find(|(_, pos)| *pos == cur_pos)           // exact match
            .or_else(|| records.iter().min_by_key(|(_, pos)| (pos - cur_pos).abs())) // closest
            .copied();                                    // fallback never needed since records not empty

        if let Some((target_ws, target_pos)) = best {
            let matched = if target_pos == cur_pos { "pos match" } else { "closest pos" };
            eprintln!(
                "  Window {:>4} \"{}\" [{}]: DB record found ({}) — workspace={}, position={}{}",
                w.id,
                w.title,
                w.app_id,
                matched,
                target_ws,
                target_pos,
                if target_ws != cur_ws || target_pos != cur_pos {
                    format!(" (current ws={}, pos={}; needs restore)", cur_ws, cur_pos)
                } else {
                    " (already matches; skipped)".to_string()
                }
            );
            if target_ws != cur_ws || target_pos != cur_pos {
                to_restore.push((w.id, w.app_id.clone(), w.title.clone(), target_ws, target_pos));
            }
        }
    }

    if to_restore.is_empty() {
        eprintln!("No windows to restore.");
        return;
    }

    // 3. Sort: first by target workspace (ascending), then by position (ascending).
    //    This fills lower workspaces first and places windows left-to-right.
    to_restore.sort_by_key(|(_, _, _, ws, pos)| (*ws, *pos));

    // 4. Apply moves. niri auto-creates workspaces when a window is moved to one.
    for (win_id, app_id, title, target_ws, target_pos) in &to_restore {
        match niri::move_window_to_workspace(*win_id, *target_ws) {
            Ok(()) => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                eprintln!("Failed to move window {}: {e}", win_id);
                continue;
            }
        }

        // 5. Position the window left-to-right on its workspace.
        //    move-column-to-index uses a 0-based index.
        let idx = (*target_pos - 1).max(0);
        if idx > 0 {
            match niri::move_focused_column_to_index(idx) {
                Ok(()) => {
                    eprintln!(
                        "Moved window {} \"{}\" [{}] → ws {} (pos {})",
                        win_id, title, app_id, target_ws, target_pos
                    );
                }
                Err(e) => {
                    eprintln!(
                        "Moved window {} \"{}\" [{}] → ws {} (column pos skipped: {e})",
                        win_id, title, app_id, target_ws
                    );
                }
            }
            thread::sleep(Duration::from_millis(150));
        } else {
            eprintln!(
                "Moved window {} \"{}\" [{}] → ws {} (pos {})",
                win_id, title, app_id, target_ws, target_pos
            );
        }
    }

    eprintln!("Restored {} window(s).", to_restore.len());
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Watch { interval, db } => run_watch(&db, interval),
        Command::Restore { db } => run_restore(&db),
    }
}
