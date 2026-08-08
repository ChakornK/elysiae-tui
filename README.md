# Elysiae TUI

Lightweight version of [Elysiae](https://github.com/elysiae-project/elysiae), a Linux game launcher for Chinese anime games. Terminal UI by default, headless CLI when you pass a subcommand.

Supports:

- bh3
- hk4e
- hkrpg
- nap

## Features

- Download, update, verify, and launch games through Proton
- Resume interrupted downloads
- Pre-download upcoming version patches before they go live
- Manage multiple voice-over language packs per game
- Auto-install GE-Proton

## Usage

### TUI mode

```sh
elysiae-tui
```

Tab between games, trigger downloads/updates, watch progress, launch. Background images render in-terminal via Unicode quadrant blocks. Settings view manages VO languages and uninstalls.

### CLI mode

```sh
elysiae-tui download hk4e --path ~/Games/genshin
elysiae-tui update hkrpg
elysiae-tui launch hk4e
elysiae-tui verify nap
elysiae-tui resume hk4e
elysiae-tui preinstall hkrpg
elysiae-tui apply-preinstall hkrpg
elysiae-tui check-update hk4e
```

`--lang` accepts: `en-us`, `ja-jp`, `zh-cn`, `zh-tw`, `ko-kr`. Multiple VO packs supported.

## Build

Requires Rust 1.92.0+.

```sh
cargo build --release
```

The release profile strips symbols and enables LTO. Output binary lands in `target/release/elysiae-tui`.

## Configuration

Lives at `~/.config/elysiae-tui/config.json`. Created on first run.

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
    }
  },
  "installed_components": {
    "proton": "GE-Proton9-22",
    "jadeite": "3.1.0"
  }
}
```

Data goes to `~/.local/share/elysiae-tui/` (logs, state, components, game installs). Cached backgrounds and quadrant data go to `~/.cache/elysiae-tui/`.

Corrupt configs get preserved as `.corrupted-{timestamp}` and replaced with defaults.

## Project layout

```
src/
├── main.rs          Entry point, logging, CLI-vs-TUI dispatch
├── cli.rs           clap definitions, headless command execution
├── app.rs           Central state: active download, modals, progress routing
├── config.rs        XDG paths, config load/save/migration
├── game.rs          GameId enum, display names, exe paths
├── operations.rs    Wraps irmin for download/update/verify
├── state.rs         DownloadState persistence for resume (JSON, atomic writes)
├── components.rs    Proton + Jadeite download/extraction, arch checks
├── launcher.rs      Game launch via Proton, env vars, log streaming
├── backgrounds.rs   Background image fetch from Chinese game API + caching
├── quadrant.rs      Unicode quadrant-block image encoding/rendering
├── transition.rs    Ripple-fade animation between backgrounds
├── postinstall.rs   Plugin and channel SDK install after game ops
├── http.rs          HTTP client with retry
├── atomic.rs        Atomic file writes, safe dir removal
├── disk.rs          Disk space checks
├── signal.rs        SIGINT handler
├── ui.rs            TUI rendering: tabs, panels, overlays, settings
└── tui/
    ├── mod.rs       Event loop, background task spawning
    ├── guard.rs     TerminalGuard RAII (raw mode, alternate screen, panic hook)
    ├── input.rs     Key dispatch
    └── actions.rs   Download/update/launch/uninstall orchestration
```

## Key dependencies

| Crate | Role |
|-------|------|
| [irmin](https://github.com/elysiae-project/irmin) | Sophon protocol download engine |
| ratatui + crossterm | Terminal UI |
| tokio | Async runtime |
| reqwest | HTTP |
| clap | CLI parsing |
| image | Background decoding/resize |

## Tests

```sh
cargo test
```

Integration tests cover CLI arg parsing. Config serialization roundtrips are property-tested with proptest.

## Notes

- One download runs at a time. Components auto-install before game operations.
- Downloads are resumable. State files track manifest hash + completed chunks. If upstream changes, stale progress is discarded.
- The TUI polls at 30fps. Progress arrives via mpsc channels. Child process stdout/stderr streams into the log view.
- Proton binaries are verified against host architecture by reading ELF headers.
