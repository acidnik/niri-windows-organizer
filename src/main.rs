mod db;
mod niri;

use clap::{Parser, Subcommand};
use std::collections::HashSet;
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

        let mut changed = 0usize;
        for w in &windows {
            let pos = w.layout.pos_in_scrolling_layout[0];
            match db::insert_if_changed(&conn, &w.app_id, &w.title, w.workspace_id, pos) {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(e) => eprintln!("DB insert error: {e}"),
            }
        }

        if changed > 0 {
            eprintln!("Recorded {changed} window change(s)");
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

    // 2. Get existing workspace indices (sorted ascending)
    let workspace_indices = match niri::get_workspace_indices() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Error getting workspaces: {e}");
            process::exit(1);
        }
    };

    // 3. For each window, find its last known (workspace, position) from DB.
    //    Only restore windows whose target differs from current state.
    //    Each entry: (window_id, target_workspace, target_position).
    let mut to_restore: Vec<(u64, i64, i64)> = Vec::new();

    for w in &windows {
        match db::find_last(&conn, &w.app_id, &w.title) {
            Ok(Some((target_ws, target_pos))) => {
                // If workspace or position changed, queue for restore
                if target_ws != w.workspace_id || target_pos != w.layout.pos_in_scrolling_layout[0] {
                    to_restore.push((w.id, target_ws, target_pos));
                }
            }
            _ => {}
        }
    }

    if to_restore.is_empty() {
        eprintln!("No windows to restore.");
        return;
    }

    // 4. Sort: first by target workspace (ascending), then by position (ascending).
    //    This way we fill lower workspaces first and place windows left-to-right.
    to_restore.sort_by_key(|(_, ws, pos)| (*ws, *pos));

    // 5. Apply moves.
    //    Before moving to workspace N, ensure workspaces 2..N-1 have at least
    //    one window. If a gap exists, redirect to the lowest empty workspace.
    let mut occupied: HashSet<i64> = HashSet::new();

    for (win_id, target_ws, target_pos) in &to_restore {
        // Find the lowest workspace idx >= 2 that is not yet occupied.
        let lowest_free = workspace_indices
            .iter()
            .copied()
            .skip_while(|idx| *idx < 2)
            .find(|idx| !occupied.contains(idx))
            .unwrap_or(2);

        let adjusted_ws = if *target_ws > lowest_free {
            lowest_free
        } else {
            *target_ws
        };

        match niri::move_window_to_workspace(*win_id, adjusted_ws) {
            Ok(()) => {
                eprintln!(
                    "Moved window {} to workspace {} (pos {}, original target ws {})",
                    win_id, adjusted_ws, target_pos, target_ws
                );
                occupied.insert(adjusted_ws);
                thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                eprintln!("Failed to move window {}: {e}", win_id);
                continue;
            }
        }

        // 6. Position the window left-to-right on its (possibly adjusted) workspace.
        //    We use move-column-to-index with (target_pos - 1) because niri uses 0-based index.
        let idx = (target_pos - 1).max(0);
        if idx > 0 {
            // Focus the window we just moved (it should be focused already)
            // and move its column to the desired index.
            match niri::move_focused_column_to_index(idx) {
                Ok(()) => {
                    eprintln!("  Positioned column at index {}", idx);
                }
                Err(e) => {
                    eprintln!("  Failed to position column: {e}");
                }
            }
            thread::sleep(Duration::from_millis(150));
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
