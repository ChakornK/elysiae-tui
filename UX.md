# Userflow Design: elysiae-tui

## Application State Machine

The TUI is a single-screen application with modal overlays. Every interaction routes through one decision tree.

```
┌─────────────────────────────────────────────────────────────────┐
│                        APPLICATION                              │
│                                                                 │
│  ┌───────────┐                         ┌──────────────┐        │
│  │  Startup  │────────────────────────▶│  Game List   │        │
│  └───────────┘                         │  (main view) │        │
│                                        └──────────────┘        │
│                                           │  │  │              │
│                    ┌──────────────────────┘  │  │              │
│                    │  's'                     │  │              │
│                    ▼                          │  │              │
│               ┌──────────┐    Esc            │  │              │
│               │ Settings │───────────────────▶  │              │
│               └──────────┘                      │              │
│                    │ Enter on "Manage VOs"       │              │
│                    ▼                             │              │
│               ┌──────────┐    Esc               │              │
│               │ VO Modal │───────────────────▶ Settings        │
│               └──────────┘                      │              │
│                                                 │              │
│                    ┌────────────────────────────┘              │
│                    │  '?'                                       │
│                    ▼                                            │
│               ┌──────────┐    any key                          │
│               │   Help   │────────────────────────▶ Game List  │
│               └──────────┘                                     │
│                                                                 │
│  ─── Modal overlays (render on top, block input) ───           │
│                                                                 │
│  ┌──────────────────┐  ┌──────────────────┐                   │
│  │  Error Modal     │  │  Confirm Dialog  │                   │
│  │  (any key)       │  │  (←/→/Enter/Esc) │                   │
│  └──────────────────┘  └──────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
```

## Per-Game State Derivation

Each game in the list has exactly one derived state. Priority order matters:

```
fn game_state(app, game) -> GameState:
    if app.download.is_some() AND dl.game_id == game:
        return Downloading (or Updating/Preinstalling based on op type)
    
    let gs = app.games[game]
    
    if gs.has_resume:
        return Resumable
    
    if gs.installed_tag.is_some():
        if gs.update_info.update_available:
            return UpdateAvailable
        else:
            return Installed
    
    return NotInstalled
```

### State → UI Mapping

| Game State | Button Label | Enter Action | Available Keys |
|-----------|-------------|-------------|----------------|
| NotInstalled | `Get Game` | `start_download()` | — |
| Resumable | `Resume` | `start_resume()` | — |
| Installed | `Launch` | `prepare_and_launch()` | `v` verify |
| UpdateAvailable | `Update` | `start_update()` | `p` preinstall, `a` apply |
| Downloading | `Downloading...` | no-op | `p` pause/resume, `c` cancel |
| Updating | `Updating...` | no-op | `p` pause/resume, `c` cancel |

## Startup Sequence

```
1. Load config from disk (or create default; migrate vo_lang→vo_langs)
2. Install panic hook (terminal restore)
3. Enter raw mode + alternate screen (TerminalGuard)
4. Initialize app state
5. For each game with an install_path in config:
   a. Read .sophon_version tag
   b. Check if exe exists
   c. Check if app state file exists OR chunks/ dir exists
   d. Set: installed_tag (if exe exists), has_resume (if state exists but not installed)
6. Load quadrant cache (instant)
7. Spawn background tasks:
   a. Sync + encode background images
   b. Check for updates on all installed games
8. Enter main event loop (game with has_resume shows "Resume" button)
```

## Main Event Loop (per tick, 33ms)

```
1. Check shutdown signal → break if triggered
2. Render frame
3. Poll background task results (images, update info)
4. Poll terminal events:
   a. Resize → clear + reload cache
   b. Key press:
      - Download controls (p/c) bypass modal stack when download active
      - VO modal (if open): Up/Down/Space/Enter/Esc
      - Confirm dialog (if open): ←/→/Enter/y/Esc/n
      - Error/status modals: any key dismisses
      - Help overlay: any key dismisses
      - Then route to view handler (GameList or Settings)
5. Drain progress channel → update_progress()
6. Drain log channel → append to launch_log
7. Check ready_to_launch flag → spawn game
8. Check should_quit → break
```

## Key Dispatch (Game List View)

```
┌─────────────────────────────────────────────────────────┐
│                  KEY PRESS                                │
│                                                          │
│  Download controls bypass all modals:                    │
│  ├── 'p' (if download active) → pause/resume            │
│  └── 'c' (if download active) → open cancel dialog      │
│                                                          │
│  Modal stack (checked in order, consumes the key):       │
│  ├── vo_modal → Up/Down/Space/Enter/Esc                  │
│  ├── confirm dialog → ←/→/Enter/y/Esc/n                 │
│  ├── error_message → any key dismisses                   │
│  ├── status_message → any key dismisses                  │
│  └── show_help → any key dismisses                       │
│                                                          │
│  View keys:                                              │
│  ├── 'q' → quit                                          │
│  ├── ←/→/Tab/BackTab → switch game                       │
│  ├── '1'-'4' → jump to game                              │
│  ├── Up/Down (if game running) → scroll log             │
│  ├── Enter → primary action (see state table above)      │
│  ├── 'v' → verify (if installed, no active download)     │
│  ├── 'p' → preinstall (if available, no active download) │
│  ├── 'a' → apply preinstall (if ready)                   │
│  ├── 's' → settings view                                 │
│  ├── '?' → help overlay                                  │
│  └── other → ignored                                     │
└─────────────────────────────────────────────────────────┘
```

## Key Dispatch (Settings View)

```
┌─────────────────────────────────────────────────────────┐
│                  KEY PRESS                                │
│                                                          │
│  ├── Up/Down → move cursor (skips headers)               │
│  ├── Enter → activate selected item:                     │
│  │   ├── "Manage VOs" → open VO modal                    │
│  │   ├── "Uninstall game" → open confirm dialog          │
│  │   ├── "Uninstall Proton" → open confirm dialog        │
│  │   └── "Uninstall Jadeite" → open confirm dialog       │
│  ├── Esc → return to Game List                           │
│  └── other → ignored                                     │
└─────────────────────────────────────────────────────────┘
```

## Confirm Dialog

```
┌─ Cancel Download ────────────────────────┐
│                                           │
│  Cancel the active download?              │
│                                           │
│  [y] Yes      [esc] No                   │
│                                           │
└───────────────────────────────────────────┘

Controls:
  ←/→     move selection between Yes and No
  Enter   activate selected button
  y       immediately confirm (shortcut)
  Esc/n   immediately dismiss

Default selection: No (prevents accidental destructive action)
```

## VO Manager Modal

```
┌─ Manage Voice-Overs ──────────────────────┐
│                                            │
│  [x] English (en-us)                       │
│  [x] Japanese (ja-jp)                      │
│  [ ] Chinese Simplified (zh-cn)            │
│  [ ] Chinese Traditional (zh-tw)           │
│  [ ] Korean (ko-kr)                        │
│                                            │
│  [space] toggle  [enter] apply  [esc]      │
└────────────────────────────────────────────┘

Controls:
  Up/Down   move cursor (wraps)
  Space     toggle enabled/disabled on cursor row
  Enter     apply changes (download new, remove old)
  Esc       discard changes and close

Rules:
  - At least one language must remain enabled
  - Toggling the last enabled language is a no-op
  - On apply: config saves immediately, new langs trigger download
```

## Settings View Layout

```
┌─ Settings ─────────────────────────────────────────────┐
│                                                         │
│  ─── Genshin Impact ────────────────────────────────    │
│    Manage VOs (2 enabled)              ← cursor here    │
│    Uninstall game                                       │
│                                                         │
│  ─── Honkai: Star Rail ─────────────────────────────    │
│    Manage VOs (1 enabled)                               │
│    Uninstall game                                       │
│                                                         │
│  ─── Components ────────────────────────────────────    │
│    Proton: GE-Proton9-22                                │
│    Uninstall Proton                                     │
│    Jadeite: 3.1.0                                       │
│    Uninstall Jadeite                                    │
│                                                         │
│  [↑/↓] navigate  [enter] select  [esc] back            │
└─────────────────────────────────────────────────────────┘

Only installed games appear. Non-selectable rows (headers, component
info) are skipped by cursor navigation. Uninstall items render red.
```

## Download Lifecycle

```
User presses Enter on a NotInstalled/Resumable/UpdateAvailable game
    │
    ▼
start_download / start_resume / start_update
    │
    ├── Set app.download = Some(ActiveDownload { game_id, handle, op_label, ... })
    │
    ▼
spawn_operation (tokio::spawn)
    │
    ├── ensure_components (Proton, Jadeite if needed)
    │   ├── Download + extract if missing/outdated
    │   ├── Progress shown via SophonProgress events
    │   └── Cancellable via handle
    │
    ├── ops.download / ops.update / ops.preinstall
    │   ├── build_installers (fetches manifest from API)
    │   ├── build_resume_context (loads state file if exists)
    │   │   ├── Manifest hash matches → resume with prior chunks
    │   │   └── Manifest hash differs → discard stale, start fresh
    │   ├── game_installer::install (irmin core)
    │   │   ├── Downloads chunks (concurrent, retried)
    │   │   ├── state_saver callback persists progress atomically
    │   │   ├── Assembles files (decompresses, verifies hashes)
    │   │   └── Writes .sophon_version tag
    │   └── On success: removes state file
    │
    ├── run_post_install (plugins + channel SDKs)
    │
    └── Send SophonProgress::Finished
            │
            ▼
        app.update_progress(Finished)
            │
            ├── gs.has_resume = false
            ├── gs.installed_tag = read from disk
            └── app.download = None
                (UI now shows "Launch" button)
```

## Cancel Flow

```
User presses 'c' during active download
    │
    ▼
Confirm dialog opens (default: No selected)
    │
    ├── User presses 'y' or selects Yes + Enter
    │
    ▼
app.finish_download()
    │
    ├── handle.cancel() → irmin stops
    ├── Check if state file exists on disk AND chunks/ dir exists AND exe missing
    │   ├── YES → gs.has_resume = true (button becomes "Resume")
    │   └── NO → gs.has_resume = false (button becomes "Get Game")
    ├── app.download = None
    └── Remove partial component archives
```

## Resume Flow

```
User presses Enter on a game with has_resume = true
    │
    ▼
start_resume()
    │
    ├── Load state file → get DownloadType
    │   ├── Fresh → Op::Download
    │   ├── Update → Op::Update
    │   └── Preinstall → Op::Preinstall
    │
    ▼
spawn_operation(op)
    │
    ▼
ops.download/update/preinstall
    │
    ├── build_resume_context()
    │   ├── Load state file
    │   ├── Compare manifest_hash with current remote
    │   │   ├── Match → pass prev_downloaded_chunks + is_resume=true
    │   │   └── Mismatch → discard chunks, start fresh
    │   └── Create new state_saver closure
    │
    └── game_installer::install (resumes from checkpoint)
```

## Launch Flow

```
User presses Enter on an Installed game
    │
    ▼
prepare_and_launch()
    │
    ├── Check: Proton available? Jadeite available (if needed)?
    │   ├── All present → app.ready_to_launch = true
    │   └── Missing → start component install with launch_on_complete = true
    │                   (progress overlay shown, game launches on Finished)
    │
    ▼ (on next loop tick, ready_to_launch checked)
launch_game()
    │
    ├── Clear launch log
    ├── Set game_running = true
    ├── Spawn: sh -c "{proton} run {jadeite?} {game_exe}"
    │   ├── Env: STEAM_COMPAT_DATA_PATH, __NV_DISABLE_EXPLICIT_SYNC=1
    │   ├── kill_on_drop(true)
    │   ├── Streams stdout/stderr → log_tx → launch_log (VecDeque)
    │   └── On exit: sends __PROCESS_EXIT__ sentinel
    │
    └── TUI remains interactive (user can browse other games)
         Game output visible via Up/Down scroll
```

## Uninstall Flow (from Settings)

```
User selects "Uninstall game" and presses Enter
    │
    ▼
Confirm dialog opens: "Uninstall {game name}?"
    │
    ├── User confirms
    │
    ▼
uninstall_game(game)
    │
    ├── safe_remove_dir_all(install_path)
    ├── Remove state file
    ├── Clear installed_tag, has_resume, update_info
    ├── Clear install_path from config
    └── Persist config atomically

User selects "Uninstall Proton/Jadeite" and presses Enter
    │
    ▼
Confirm dialog opens: "Uninstall {component}?"
    │
    ├── User confirms
    │
    ▼
uninstall_component(name)
    │
    ├── Proton: remove proton/, proton-data/, proton.tag
    ├── Jadeite: remove jadeite/, jadeite.tag
    ├── Clear config.installed_components entry
    └── Persist config atomically
```

## Signal Handling

```
SIGINT (Ctrl+C) received
    │
    ├── shutdown_rx becomes true
    ├── Main loop checks at top of iteration
    ├── If download active: handle.cancel()
    │   └── state_saver has already persisted progress
    ├── Break from loop
    ├── TerminalGuard::drop() restores terminal
    └── Process exits cleanly
```

## Error Recovery

| Scenario | What happens |
|----------|-------------|
| Network timeout during download | irmin retries 5x internally; if all fail, Error event sent, state file preserved |
| Panic in any code | Panic hook restores terminal, prints panic info |
| `?` return inside TUI function | TerminalGuard Drop restores terminal |
| Disk full during download | Write fails, Error event sent, state file has progress up to last successful chunk |
| Corrupt config.json | Preserved as `.corrupted-{ts}`, fresh default created |
| Game process crashes | Exit code shown in log, `game_running` cleared on `__PROCESS_EXIT__` |
| Uninstall fails (permissions) | Error modal shown with OS error message |
| VO download fails | Error modal shown, previously-enabled languages remain |

## Visual Layout (Game List)

```
┌─ Terminal ────────────────────────────────────────────────┐
│                                                           │
│  ┌─ Tab Bar ──────────────────────────────────────────┐  │
│  │  [1] Honkai Impact 3rd  [2] Genshin Impact  ...   │  │
│  └────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─ Background (quadrant-encoded image) ──────────────┐  │
│  │                                                     │  │
│  │   ┌─ Info Panel (semi-transparent) ─────────────┐  │  │
│  │   │  Genshin Impact                              │  │  │
│  │   │  Version: 6.7.0                              │  │  │
│  │   │                                              │  │  │
│  │   │  [Update available badge if applicable]      │  │  │
│  │   └──────────────────────────────────────────────┘  │  │
│  │                                                     │  │
│  └─────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─ Action Bar ───────────────────────────────────────┐  │
│  │  [⏎] Launch              [?] help  [q] quit       │  │
│  └────────────────────────────────────────────────────┘  │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

## Data Flow Diagram

```
              ┌─────────────┐
              │   Config    │ ← persisted to disk (atomic write)
              │  (JSON)     │ ← vo_langs: Vec<String> per game
              └──────┬──────┘
                     │ load at startup (migrates vo_lang → vo_langs)
                     ▼
┌──────────┐    ┌─────────┐    ┌───────────────┐
│ irmin    │◀───│   App   │───▶│  TUI Render   │
│ (crate)  │    │  State  │    │  (ui.rs)      │
└──────────┘    └─────────┘    └───────────────┘
     │               ▲
     │ progress      │ key events
     ▼               │
┌──────────┐    ┌─────────┐
│  mpsc    │───▶│  Input  │
│ channel  │    │ Handler │
└──────────┘    └─────────┘

     ┌──────────────────┐
     │  State File      │ ← written by state_saver on each chunk batch
     │  (.sophon_state) │ ← read on resume
     │  (atomic write)  │ ← deleted on success
     └──────────────────┘
```

## Config Schema

```json
{
  "version": 1,
  "selected_game": "hk4e",
  "auto_update": true,
  "auto_preload": true,
  "games": {
    "hk4e": {
      "vo_langs": ["en-us", "ja-jp"],
      "install_path": "/home/user/.local/share/elysiae-tui/games/hk4e"
    },
    "hkrpg": {
      "vo_langs": ["en-us"],
      "install_path": "/home/user/.local/share/elysiae-tui/games/hkrpg"
    }
  },
  "installed_components": {
    "proton": "GE-Proton9-22",
    "jadeite": "3.1.0"
  }
}
```

Backward compatible: old configs with `"vo_lang": "en-us"` auto-migrate to `"vo_langs": ["en-us"]` on load.

## Session Lifecycle

```
┌─ First Launch ──────────────────────────────────────────┐
│ 1. No config → create default                           │
│ 2. No games installed → all show "Get Game"             │
│ 3. No backgrounds cached → blank until sync completes   │
│ 4. User selects game, presses Enter                     │
│ 5. Default install path auto-assigned                   │
│ 6. Proton downloaded → game downloaded → ready          │
└─────────────────────────────────────────────────────────┘

┌─ Normal Launch ─────────────────────────────────────────┐
│ 1. Config loaded with paths + component versions        │
│ 2. Installed games show "Launch" or "Update"            │
│ 3. Background images load from quadrant cache (<1ms)    │
│ 4. Update check runs in background                      │
│ 5. User presses Enter → game launches immediately       │
└─────────────────────────────────────────────────────────┘

┌─ Interrupted Session ───────────────────────────────────┐
│ 1. State file found on disk for a game                  │
│ 2. Game shows "Resume" button in the game list          │
│ 3. User presses Enter → resumes from last checkpoint    │
└─────────────────────────────────────────────────────────┘
```
