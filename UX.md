# Userflow Design: elysiae-tui

## Application State Machine

The entire TUI is a single-screen application with modal overlays. Every interaction routes through one decision tree.

```
┌─────────────────────────────────────────────────────────────────┐
│                        APPLICATION                              │
│                                                                 │
│  ┌───────────┐                         ┌──────────────┐        │
│  │  Startup  │────────────────────────▶│  Game List   │        │
│  └───────────┘                         │  (main view) │        │
│       │                                └──────────────┘        │
│       │ has resume state?                 │  │  │  │           │
│       ▼                                   │  │  │  │           │
│  ┌───────────┐     dismiss                │  │  │  │           │
│  │  Resume   │──────────────────────────▶─┘  │  │  │           │
│  │  Prompt   │                               │  │  │           │
│  └───────────┘                               │  │  │           │
│                                              │  │  │           │
│                    ┌─────────────────────────┘  │  │           │
│                    │  's'                        │  │           │
│                    ▼                             │  │           │
│               ┌──────────┐    Esc               │  │           │
│               │ Settings │──────────────────────▶  │           │
│               └──────────┘                         │           │
│                                                    │           │
│                    ┌───────────────────────────────┘           │
│                    │  '?'                                       │
│                    ▼                                            │
│               ┌──────────┐    any key                          │
│               │   Help   │────────────────────────▶ Game List  │
│               └──────────┘                                     │
│                                                                 │
│  ─── Modal overlays (render on top, block input) ───           │
│                                                                 │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐   │
│  │  Error Modal   │  │ Cancel Confirm │  │ Status Message │   │
│  │  (any key)     │  │  (y/n)         │  │  (any key)     │   │
│  └────────────────┘  └────────────────┘  └────────────────┘   │
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
1. Load config from disk (or create default)
2. Install panic hook (terminal restore)
3. Enter raw mode + alternate screen (TerminalGuard)
4. Initialize app state
5. For each game with an install_path in config:
   a. Read .sophon_version tag
   b. Check if exe exists
   c. Check if resume state file exists OR chunks/ dir exists
   d. Set: installed_tag (if exe exists), has_resume (if state exists but not installed)
6. Load quadrant cache (instant)
7. Spawn background tasks:
   a. Sync + encode background images
   b. Check for updates on all installed games
8. Check for resume state → show prompt if found
9. Enter main event loop
```

## Main Event Loop (per tick, 33ms)

```
1. Check shutdown signal → break if triggered
2. Render frame
3. Poll background task results (images, update info)
4. Poll terminal events:
   a. Resize → clear + reload cache
   b. Key press:
      - Route through modal stack (error > status > resume prompt > help > cancel confirm)
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
│  Is download active for ANY game?                        │
│  ├── YES: intercept 'p' (pause/resume) and 'c' (cancel) │
│  └── NO: fall through                                    │
│                                                          │
│  Modal stack (checked first, consumes the key):          │
│  ├── error_message → dismiss, continue                   │
│  ├── status_message → dismiss, continue                  │
│  ├── show_resume_prompt → y=resume, other=dismiss        │
│  ├── show_help → any key dismisses                       │
│  └── show_cancel_confirm → y=cancel dl, other=dismiss    │
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

## Download Lifecycle

```
User presses Enter on a NotInstalled/Resumable/UpdateAvailable game
    │
    ▼
start_download / start_resume / start_update
    │
    ├── Set app.download = Some(ActiveDownload { game_id, handle, ... })
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
show_cancel_confirm = true
    │
    ├── UI renders "Cancel download? (y/n)" modal
    │
    ▼
User presses 'y'
    │
    ▼
app.finish_download()
    │
    ├── handle.cancel() → irmin stops
    ├── Check if state file exists on disk
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
| Two instances launched | Second instance fails to acquire lockfile, exits with message |

## Visual Layout

```
┌─ Terminal ────────────────────────────────────────────────┐
│                                                           │
│  ┌─ Tab Bar ──────────────────────────────────────────┐  │
│  │  [bh3]  [genshin]  [starrail]  [zzz]              │  │
│  └────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─ Background (quadrant-encoded image) ──────────────┐  │
│  │                                                     │  │
│  │   ┌─ Info Panel (semi-transparent) ─────────────┐  │  │
│  │   │  Game Title                                  │  │  │
│  │   │  Version: 6.7.0  (or "Not installed")       │  │  │
│  │   │                                              │  │  │
│  │   │  [Update available badge if applicable]      │  │  │
│  │   └──────────────────────────────────────────────┘  │  │
│  │                                                     │  │
│  └─────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─ Action Bar ───────────────────────────────────────┐  │
│  │  [⏎] Resume              [?] help  [q] quit       │  │
│  └────────────────────────────────────────────────────┘  │
│                                                           │
└───────────────────────────────────────────────────────────┘

┌─ Download Overlay (when active) ──────────────────────────┐
│                                                            │
│  Downloading bh3                                           │
│  ████████████████░░░░░░░░░░░░░  45.2%                     │
│  1.2 GB / 2.7 GB  │  85.3 MB/s  │  ETA 18s               │
│                                                            │
│  [p] pause  [c] cancel                                     │
│                                                            │
└────────────────────────────────────────────────────────────┘

┌─ Launch Log (when game running) ──────────────────────────┐
│  bh3 output:                                               │
│  > Proton 9.0-4                                            │
│  > esync: up                                               │
│  > ...                                                     │
│                                                            │
│  [↑/↓] scroll                                              │
└────────────────────────────────────────────────────────────┘
```

## Data Flow Diagram

```
              ┌─────────────┐
              │   Config    │ ← persisted to disk (atomic write)
              │  (JSON)     │
              └──────┬──────┘
                     │ load at startup
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
│ 2. Startup prompt: "Interrupted download found. Resume?"│
│ 3. 'y' → resumes from last checkpoint                   │
│ 4. Dismiss → game shows "Resume" button permanently     │
│    until user presses Enter (resumes) or deletes state  │
└─────────────────────────────────────────────────────────┘
```
