# Userflow Design: elysiae-tui

## Application State Machine

Single screen with modal overlays. All input feeds one decision tree.

```mermaid
stateDiagram-v2
    [*] --> GameList : Startup

    GameList --> Settings : s
    GameList --> Help : ?

    Settings --> GameList : Esc
    Settings --> VOModal : Enter on "Manage VOs"

    VOModal --> Settings : Esc

    Help --> GameList : any key

    state "Modal Overlays" as Modals {
        ErrorModal : any key dismisses
        ConfirmDialog : ←/→/Enter/Esc
    }

    GameList : (main view)
```

## Per-Game State Derivation

Each game resolves to one state. Checked top-to-bottom, first match wins:

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

```mermaid
sequenceDiagram
    participant App
    participant Config
    participant Terminal
    participant Games
    participant Background

    App->>Config: Load from disk (or create default)
    Note over Config: Migrate vo_lang → vo_langs
    App->>Terminal: Install panic hook (terminal restore)
    App->>Terminal: Enter raw mode + alternate screen (TerminalGuard)
    App->>App: Initialize app state

    loop For each game with install_path
        App->>Games: Read .sophon_version tag
        App->>Games: Check if exe exists
        App->>Games: Check if state file OR chunks/ dir exists
        Note over Games: installed_tag = exe exists<br/>has_resume = state exists & not installed
    end

    App->>App: Load quadrant cache (instant)
    App->>Background: Spawn: sync + encode background images
    App->>Background: Spawn: check updates on installed games
    App->>App: Enter main event loop
    Note over App: Games with has_resume show "Resume" button
```

## Main Event Loop (per tick, 33ms)

```mermaid
flowchart TD
    Start([Loop Start]) --> Shutdown{Shutdown signal?}
    Shutdown -->|Yes| Break([Break])
    Shutdown -->|No| Render[Render frame]
    Render --> PollBG[Poll background task results<br/>images, update info]
    PollBG --> PollEvents[Poll terminal events]

    PollEvents --> Resize{Resize?}
    Resize -->|Yes| ClearCache[Clear + reload cache]
    Resize -->|No| KeyPress{Key press?}

    KeyPress --> DL{Download active?<br/>p/c pressed?}
    DL -->|Yes| DLCtrl[Handle download control]
    DL -->|No| VO{VO modal open?}
    VO -->|Yes| VOHandle[Up/Down/Space/Enter/Esc]
    VO -->|No| Confirm{Confirm dialog?}
    Confirm -->|Yes| ConfHandle[←/→/Enter/y/Esc/n]
    Confirm -->|No| ErrModal{Error/Status modal?}
    ErrModal -->|Yes| Dismiss1[Any key dismisses]
    ErrModal -->|No| Help{Help overlay?}
    Help -->|Yes| Dismiss2[Any key dismisses]
    Help -->|No| Route[Route to view handler<br/>GameList or Settings]

    ClearCache --> Drain
    DLCtrl --> Drain
    VOHandle --> Drain
    ConfHandle --> Drain
    Dismiss1 --> Drain
    Dismiss2 --> Drain
    Route --> Drain

    Drain[Drain progress channel → update_progress] --> DrainLog[Drain log channel → append to launch_log]
    DrainLog --> Launch{ready_to_launch?}
    Launch -->|Yes| Spawn[Spawn game]
    Launch -->|No| Quit{should_quit?}
    Spawn --> Quit
    Quit -->|Yes| Break
    Quit -->|No| Start
```

## Key Dispatch (Game List View)

```mermaid
flowchart TD
    KP[Key Press] --> DC{Download active?}

    DC -->|Yes, 'p'| PAUSE[Pause/Resume]
    DC -->|Yes, 'c'| CANCEL[Open Cancel Dialog]
    DC -->|No / other key| MS{Modal open?}

    subgraph Priority 1: Download Controls
        PAUSE
        CANCEL
    end

    MS -->|vo_modal| VO[Up/Down/Space/Enter/Esc]
    MS -->|confirm dialog| CD[←/→/Enter/y/Esc/n]
    MS -->|error_message| EM[Any key dismisses]
    MS -->|status_message| SM[Any key dismisses]
    MS -->|show_help| SH[Any key dismisses]
    MS -->|None| VK{View Keys}

    subgraph Priority 2: Modal Stack
        VO
        CD
        EM
        SM
        SH
    end

    VK -->|q| QUIT[Quit]
    VK -->|←/→/Tab/BackTab| SWITCH[Switch Game]
    VK -->|1-4| JUMP[Jump to Game]
    VK -->|Up/Down| SCROLL[Scroll Log]
    VK -->|Enter| PRIMARY[Primary Action]
    VK -->|v| VERIFY[Verify]
    VK -->|p| PRE[Preinstall]
    VK -->|a| APPLY[Apply Preinstall]
    VK -->|s| SETTINGS[Settings View]
    VK -->|?| HELP[Help Overlay]
    VK -->|other| IGNORE[Ignored]

    subgraph Priority 3: View Keys
        QUIT
        SWITCH
        JUMP
        SCROLL
        PRIMARY
        VERIFY
        PRE
        APPLY
        SETTINGS
        HELP
        IGNORE
    end
```

## Key Dispatch (Settings View)

```mermaid
flowchart TD
    A[KEY PRESS] --> B{Key?}
    B -->|Up/Down| C[Move cursor\nskips headers]
    B -->|Enter| D{Selected item?}
    B -->|Esc| E[Return to Game List]
    B -->|Other| F[Ignored]
    D -->|Manage VOs| G[Open VO modal]
    D -->|Uninstall game| H[Open confirm dialog]
    D -->|Uninstall Proton| I[Open confirm dialog]
    D -->|Uninstall Jadeite| J[Open confirm dialog]
```

## Confirm Dialog

```mermaid
block-beta
  columns 1
  block:dialog["Cancel Download"]
    columns 2
    msg["Cancel the active download?"]:2
    space:2
    yes["[y] Yes"]
    no["[esc] No"]
  end
```

Controls:
- `←/→` move selection between Yes and No
- `Enter` activate selected button
- `y` immediately confirm (shortcut)
- `Esc/n` immediately dismiss

Default selection: No (prevents accidental destructive action)

## VO Manager Modal

```mermaid
block-beta
  columns 1
  block:modal["Manage Voice-Overs"]
    columns 1
    en["[x] English (en-us)"]
    ja["[x] Japanese (ja-jp)"]
    zhcn["[ ] Chinese Simplified (zh-cn)"]
    zhtw["[ ] Chinese Traditional (zh-tw)"]
    ko["[ ] Korean (ko-kr)"]
    space[" "]
    footer["[space] toggle  [enter] apply  [esc]"]
  end
```

Rules:
- At least one language must remain enabled
- Toggling the last enabled language is a no-op
- On apply: config saves immediately, new langs trigger download

## Settings View Layout

```mermaid
block-beta
  columns 1
  block:settings["Settings"]
    columns 1
    gh["─── Genshin Impact ───"]
    gvo["Manage VOs (2 enabled)  ← cursor"]
    gun["Uninstall game"]
    sh["─── Honkai: Star Rail ───"]
    svo["Manage VOs (1 enabled)"]
    sun["Uninstall game"]
    ch["─── Components ───"]
    cp["Proton: GE-Proton9-22"]
    cpu["Uninstall Proton"]
    cj["Jadeite: 3.1.0"]
    cju["Uninstall Jadeite"]
    nav["[↑/↓] navigate  [enter] select  [esc] back"]
  end
```

Only installed games appear. Cursor skips non-selectable rows (headers, component info). Uninstall items render red.

## Download Lifecycle

```mermaid
flowchart TD
    A["User presses Enter on<br/>NotInstalled / Resumable / UpdateAvailable"]
    A --> B["start_download / start_resume / start_update"]
    B --> C["Set app.download = Some(ActiveDownload)"]
    C --> D["spawn_operation (tokio::spawn)"]

    D --> E

    subgraph ensure["ensure_components (Proton, Jadeite)"]
        E["Download + extract if missing/outdated"]
        E --> E1["Progress via SophonProgress events"]
        E --> E2["Cancellable via handle"]
    end

    ensure --> F

    subgraph install["ops.download / ops.update / ops.preinstall"]
        F["build_installers (fetch manifest from API)"]
        F --> G["build_resume_context (load state file)"]
        G -->|hash matches| G1["Resume with prior chunks"]
        G -->|hash differs| G2["Discard stale, start fresh"]
        G1 --> H
        G2 --> H
        H["game_installer::install (irmin core)"]
        H --> H1["Download chunks (concurrent, retried)"]
        H --> H2["state_saver persists progress atomically"]
        H --> H3["Assemble files (decompress, verify hashes)"]
        H --> H4["Write .sophon_version tag"]
        H1 & H2 & H3 & H4 --> I["On success: remove state file"]
    end

    install --> J["run_post_install (plugins + channel SDKs)"]
    J --> K["Send SophonProgress::Finished"]

    K --> L

    subgraph cleanup["app.update_progress(Finished)"]
        L["gs.has_resume = false"]
        L --> M["gs.installed_tag = read from disk"]
        M --> N["app.download = None"]
    end

    N --> O(["UI shows Launch button"])
```

## Cancel Flow

```mermaid
flowchart TD
    A([User presses 'c' during active download]) --> B(Confirm dialog opens<br/>default: No selected)
    B --> C{User presses 'y' or<br/>selects Yes + Enter?}
    C -->|Yes| D(app.finish_download)
    D --> E(handle.cancel → irmin stops)
    D --> F{state file exists AND<br/>chunks/ dir exists AND<br/>exe missing?}
    F -->|YES| G(gs.has_resume = true<br/>button becomes 'Resume')
    F -->|NO| H(gs.has_resume = false<br/>button becomes 'Get Game')
    D --> I(app.download = None)
    D --> J(Remove partial component archives)
```

## Resume Flow

```mermaid
flowchart TD
    A([User presses Enter on game<br/>with has_resume = true]) --> B(start_resume)
    B --> C(Load state file)
    C --> D{DownloadType?}
    D -->|Fresh| E(Op::Download)
    D -->|Update| F(Op::Update)
    D -->|Preinstall| G(Op::Preinstall)
    E --> H(spawn_operation op)
    F --> H
    G --> H
    H --> I(ops.download / update / preinstall)
    I --> J(build_resume_context)
    J --> K(Load state file)
    J --> L{manifest_hash vs<br/>current remote?}
    L -->|Match| M(Pass prev_downloaded_chunks<br/>+ is_resume = true)
    L -->|Mismatch| N(Discard chunks, start fresh)
    J --> O(Create new state_saver closure)
    M --> P(game_installer::install<br/>resumes from checkpoint)
    N --> P
    O --> P
```

## Launch Flow

```mermaid
flowchart TD
    A([User presses Enter on Installed game]) --> B[prepare_and_launch]
    B --> C{Proton/Jadeite available?}
    C -->|All present| D[ready_to_launch = true]
    C -->|Missing| E[Start component install<br/>launch_on_complete = true]
    E --> F[Progress overlay shown]
    F --> D
    D -->|Next loop tick| G[launch_game]
    G --> H[Clear launch log]
    H --> I[Set game_running = true]
    I --> J["Spawn: sh -c {proton} run {jadeite?} {game_exe}"]
    J --> K[STEAM_COMPAT_DATA_PATH<br/>__NV_DISABLE_EXPLICIT_SYNC=1]
    J --> L[kill_on_drop = true]
    J --> M[Stream stdout/stderr → log_tx → launch_log]
    J --> N[On exit: send __PROCESS_EXIT__ sentinel]
    G --> O[/TUI remains interactive<br/>Game output visible via Up/Down scroll/]
```

## Uninstall Flow (from Settings)

```mermaid
flowchart TD
    A1([User selects 'Uninstall game']) --> B1{"Confirm: Uninstall {game name}?"}
    B1 -->|User confirms| C1[uninstall_game]
    C1 --> D1[safe_remove_dir_all install_path]
    D1 --> E1[Remove state file]
    E1 --> F1[Clear installed_tag, has_resume, update_info]
    F1 --> G1[Clear install_path from config]
    G1 --> H1[Persist config atomically]

    A2([User selects 'Uninstall Proton/Jadeite']) --> B2{"Confirm: Uninstall {component}?"}
    B2 -->|User confirms| C2[uninstall_component]
    C2 --> D2{Component type?}
    D2 -->|Proton| E2[Remove proton/, proton-data/, proton.tag]
    D2 -->|Jadeite| F2[Remove jadeite/, jadeite.tag]
    E2 --> G2[Clear config.installed_components entry]
    F2 --> G2
    G2 --> H2[Persist config atomically]
```

## Signal Handling

```mermaid
flowchart TD
    A([SIGINT / Ctrl+C received]) --> B[shutdown_rx becomes true]
    B --> C[Main loop checks at top of iteration]
    C --> D{Download active?}
    D -->|Yes| E[handle.cancel]
    E --> F[state_saver has already persisted progress]
    F --> G[Break from loop]
    D -->|No| G
    G --> H["TerminalGuard::drop() restores terminal"]
    H --> I([Process exits cleanly])
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

```mermaid
block-beta
  columns 1
  block:terminal["Terminal"]
    columns 1
    block:tabbar["Tab Bar"]
      columns 4
      tab1["[1] Honkai Impact 3rd"]
      tab2["[2] Genshin Impact"]
      tab3["[3] ..."]
      tab4["[4] ..."]
    end
    block:background["Background (quadrant-encoded image)"]
      columns 1
      space
      block:infopanel["Info Panel (semi-transparent)"]
        columns 1
        title["Genshin Impact"]
        version["Version: 6.7.0"]
        badge["[Update available badge if applicable]"]
      end
      space
    end
    block:actionbar["Action Bar"]
      columns 3
      launch["[⏎] Launch"]
      help["[?] help"]
      quit["[q] quit"]
    end
  end
```

## Data Flow Diagram

```mermaid
flowchart TD
    Config["Config (JSON)<br/>vo_langs: Vec&lt;String&gt; per game"]
    AppState["App State"]
    irmin["irmin (crate)"]
    TUI["TUI Render (ui.rs)"]
    mpsc["mpsc channel"]
    Input["Input Handler"]
    StateFile["State File (.sophon_state)"]

    Config -->|"load at startup<br/>(migrates vo_lang → vo_langs)"| AppState
    AppState --> irmin
    AppState --> TUI
    irmin -->|progress| mpsc
    mpsc --> Input
    Input -->|key events| AppState

    StateFile -.-|"written on each chunk batch<br/>read on resume<br/>deleted on success"| irmin
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

```mermaid
stateDiagram-v2
    state "First Launch" as FL {
        [*] --> NoConfig: No config found
        NoConfig --> CreateDefault: Create default config
        CreateDefault --> NoGames: All games show "Get Game"
        NoGames --> BlankBG: No backgrounds cached
        BlankBG --> UserSelect: User selects game, presses Enter
        UserSelect --> AutoPath: Default install path auto-assigned
        AutoPath --> Ready: Proton downloaded → game downloaded
    }

    state "Normal Launch" as NL {
        [*] --> ConfigLoaded: Config with paths + versions
        ConfigLoaded --> ShowStatus: Installed → "Launch" or "Update"
        ShowStatus --> CacheHit: Backgrounds from quadrant cache (<1ms)
        CacheHit --> UpdateCheck: Update check runs in background
        UpdateCheck --> LaunchNow: User presses Enter → launches immediately
    }

    state "Interrupted Session" as IS {
        [*] --> StateFound: State file found on disk
        StateFound --> ShowResume: Game shows "Resume" button
        ShowResume --> Resume: User presses Enter → resumes from checkpoint
    }
```
