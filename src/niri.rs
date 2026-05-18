use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Window {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub workspace_id: i64,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_urgent: bool,
    pub pid: u64,
    pub layout: Layout,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Layout {
    pub pos_in_scrolling_layout: Vec<i64>,
}

/// Fetch all windows from niri via `niri msg --json windows`.
pub fn get_windows() -> Result<Vec<Window>, String> {
    let output = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .output()
        .map_err(|e| format!("Failed to run niri msg: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("niri msg failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {e}"))
}

/// Move a specific window to a workspace by its id.
///
/// niri requires focusing the window before moving it.
pub fn move_window_to_workspace(window_id: u64, workspace: i64) -> Result<(), String> {
    // Step 1: focus the window
    let focus = Command::new("niri")
        .args([
            "msg",
            "action",
            "focus-window",
            &format!("--id={}", window_id),
        ])
        .output()
        .map_err(|e| format!("Failed to focus window: {e}"))?;

    if !focus.status.success() {
        let stderr = String::from_utf8_lossy(&focus.stderr);
        return Err(format!("niri focus-window failed: {stderr}"));
    }

    // Step 2: move to workspace
    let r#move = Command::new("niri")
        .args([
            "msg",
            "action",
            "move-window-to-workspace",
            &workspace.to_string(),
        ])
        .output()
        .map_err(|e| format!("Failed to move window: {e}"))?;

    if !r#move.status.success() {
        let stderr = String::from_utf8_lossy(&r#move.stderr);
        return Err(format!("niri move-window-to-workspace failed: {stderr}"));
    }

    Ok(())
}

/// Move the focused window's column to a specific index on its workspace (0-based).
pub fn move_focused_column_to_index(index: i64) -> Result<(), String> {
    let output = Command::new("niri")
        .args([
            "msg",
            "action",
            "move-column-to-index",
            &index.to_string(),
        ])
        .output()
        .map_err(|e| format!("Failed to move column to index: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("niri move-column-to-index failed: {stderr}"));
    }

    Ok(())
}

/// Get current index of each existing workspace.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Workspace {
    pub id: i64,
    pub idx: i64,
    pub name: Option<String>,
    pub is_active: bool,
    pub output: String,
    pub active_window_id: Option<u64>,
}

pub fn get_workspace_indices() -> Result<Vec<i64>, String> {
    let output = Command::new("niri")
        .args(["msg", "--json", "workspaces"])
        .output()
        .map_err(|e| format!("Failed to run niri msg workspaces: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("niri msg workspaces failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let workspaces: Vec<Workspace> =
        serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {e}"))?;

    let mut indices: Vec<i64> = workspaces.into_iter().map(|w| w.idx).collect();
    indices.sort();
    Ok(indices)
}
