# niri-windows-organizer

A Rust CLI tool that remembers and restores window positions on the [niri](https://github.com/YaLTeR/niri) Wayland compositor.

It runs in the background, periodically saving each window's workspace and horizontal position to a SQLite database. After a restart (when niri puts everything on workspace 1), it redistributes windows back to their last known workspaces and column order.

## Usage

### Watch mode — save positions periodically

```bash
niri-windows-organizer watch
```

Options:

- `--interval` — polling interval in seconds (default: `10`)
- `--db` — path to the SQLite database (default: `~/.local/share/niri-windows-organizer/history.db`)

On startup, records older than 1 year are pruned automatically.

### Restore mode — restore windows to saved positions

```bash
niri-windows-organizer restore
```

This does the following:

1. Queries the database for the last known workspace and column position of each currently open window.
2. Sorts windows first by target workspace (ascending), then by horizontal position (left to right).
3. Moves windows to their workspaces — lower workspaces are filled first to avoid gaps.
4. Positions each window at its saved column index using `move-column-to-index`.

## How it works

The tool communicates with niri through `niri msg --json windows` to get the current window list. Each window record stores:

- **app_id** — application identifier (e.g. `firefox`, `org.wezfurlong.wezterm`)
- **title** — window title
- **workspace** — workspace number (1-based)
- **position** — column position on the workspace (1-indexed, left to right)

A new database record is written only when the workspace or position actually changes compared to the previous record for the same (app_id, title) pair.

## Background launch

To start the watcher automatically with niri, add this to `~/.config/niri/config.kdl`:

```
spawn-at-startup "niri-windows-organizer" "watch"
```
