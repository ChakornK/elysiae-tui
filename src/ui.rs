use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, View};
use crate::game::GameId;
use irmin::SophonProgress;

/// Renders the full TUI frame based on current application state.
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new("elysiae-cli")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(title, chunks[0]);

    match app.current_view {
        View::GameList => draw_game_list(frame, app, chunks[1]),
        View::GameDetail => draw_game_detail(frame, app, chunks[1]),
        View::Downloading => draw_downloading(frame, app, chunks[1]),
        View::Settings => draw_settings(frame, app, chunks[1]),
    }

    let keybinds = match app.current_view {
        View::GameList => "q=quit  enter=select  s=settings",
        View::GameDetail => "esc=back  d=download  u=update  l=launch  v=verify  p=preinstall",
        View::Downloading => "p=pause  r=resume  c=cancel",
        View::Settings => "esc=back",
    };
    let bottom = Paragraph::new(keybinds)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(bottom, chunks[2]);
}

fn draw_game_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = GameId::ALL
        .iter()
        .enumerate()
        .map(|(i, game)| {
            let status = app
                .games
                .get(game)
                .and_then(|s| s.installed_tag.as_deref())
                .unwrap_or("Not installed");
            let content = format!("  {} - {}", game.display_name(), status);
            let style = if i == app.game_list_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Games").borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn draw_game_detail(frame: &mut Frame, app: &App, area: Rect) {
    let game = app.active_game;
    let status = app.games.get(&game);
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("Game: ", Style::default().fg(Color::Gray)),
        Span::styled(
            game.display_name(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(gs) = status {
        if let Some(ref tag) = gs.installed_tag {
            lines.push(Line::from(vec![
                Span::styled("Version: ", Style::default().fg(Color::Gray)),
                Span::raw(tag.as_str()),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "Not installed",
                Style::default().fg(Color::Red),
            )));
        }

        if let Some(ref info) = gs.update_info {
            if info.update_available {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("Update: ", Style::default().fg(Color::Green)),
                    Span::raw(&info.remote_tag),
                    Span::raw(format!(" ({})", format_bytes(info.update_compressed_size))),
                ]));
            }
            if info.preinstall_available {
                let tag = info.preinstall_tag.as_deref().unwrap_or("unknown");
                let dl = if info.preinstall_downloaded {
                    " [downloaded]"
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::styled("Preinstall: ", Style::default().fg(Color::Magenta)),
                    Span::raw(tag),
                    Span::raw(dl),
                ]));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Not installed",
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled("Actions:", Style::default().fg(Color::Cyan)));

    let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
    let has_update = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.update_available);
    let has_preinstall = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.preinstall_available && !i.preinstall_downloaded);

    if !installed {
        lines.push(Line::raw("  [d] Download"));
    }
    if has_update {
        lines.push(Line::raw("  [u] Update"));
    }
    if installed {
        lines.push(Line::raw("  [l] Launch"));
        lines.push(Line::raw("  [v] Verify"));
    }
    if has_preinstall {
        lines.push(Line::raw("  [p] Preinstall"));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(game.display_name())
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, area);
}

fn draw_downloading(frame: &mut Frame, app: &App, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let progress = app.download.as_ref().map(|d| &d.progress);
    let (label, ratio) = match progress {
        Some(SophonProgress::Downloading {
            downloaded_bytes,
            total_bytes,
            speed_bps,
            eta_seconds,
        }) => {
            let pct = if *total_bytes > 0 {
                *downloaded_bytes as f64 / *total_bytes as f64
            } else {
                0.0
            };
            (
                format!(
                    "{:.1}% - {}/s - ETA {}",
                    pct * 100.0,
                    format_bytes(*speed_bps as u64),
                    format_eta(*eta_seconds)
                ),
                pct,
            )
        }
        Some(SophonProgress::Paused {
            downloaded_bytes,
            total_bytes,
        }) => {
            let pct = if *total_bytes > 0 {
                *downloaded_bytes as f64 / *total_bytes as f64
            } else {
                0.0
            };
            (format!("Paused - {:.1}%", pct * 100.0), pct)
        }
        Some(SophonProgress::FetchingManifest) => ("Fetching manifest...".to_owned(), 0.0),
        Some(SophonProgress::CalculatingDownloads {
            checked_files,
            total_files,
        }) => {
            let pct = if *total_files > 0 {
                *checked_files as f64 / *total_files as f64
            } else {
                0.0
            };
            (
                format!("Calculating: {}/{} files", checked_files, total_files),
                pct,
            )
        }
        Some(SophonProgress::Assembling {
            assembled_files,
            total_files,
        }) => {
            let pct = if *total_files > 0 {
                *assembled_files as f64 / *total_files as f64
            } else {
                0.0
            };
            (
                format!("Assembling: {}/{} files", assembled_files, total_files),
                pct,
            )
        }
        Some(SophonProgress::Verifying {
            scanned_files,
            total_files,
            error_count,
        }) => {
            let pct = if *total_files > 0 {
                *scanned_files as f64 / *total_files as f64
            } else {
                0.0
            };
            (
                format!(
                    "Verifying: {}/{} ({} errors)",
                    scanned_files, total_files, error_count
                ),
                pct,
            )
        }
        Some(SophonProgress::CheckingFiles {
            checked_files,
            total_files,
        }) => {
            let pct = if *total_files > 0 {
                *checked_files as f64 / *total_files as f64
            } else {
                0.0
            };
            (
                format!("Checking: {}/{} files", checked_files, total_files),
                pct,
            )
        }
        Some(SophonProgress::ApplyingPreinstall {
            applied_files,
            total_files,
        }) => {
            let pct = if *total_files > 0 {
                *applied_files as f64 / *total_files as f64
            } else {
                0.0
            };
            (
                format!("Applying: {}/{} files", applied_files, total_files),
                pct,
            )
        }
        Some(SophonProgress::InstallingPlugins { current_plugin, .. }) => {
            (format!("Installing plugin: {}", current_plugin), 0.0)
        }
        Some(SophonProgress::DownloadingPlugin {
            name,
            downloaded_bytes,
            total_bytes,
        }) => {
            let pct = if *total_bytes > 0 {
                *downloaded_bytes as f64 / *total_bytes as f64
            } else {
                0.0
            };
            (format!("Plugin {}: {:.1}%", name, pct * 100.0), pct)
        }
        Some(SophonProgress::Finished) => ("Complete!".to_owned(), 1.0),
        Some(SophonProgress::Error { message }) => (format!("Error: {}", message), 0.0),
        Some(SophonProgress::Warning { message }) => (format!("Warning: {}", message), 0.0),
        _ => ("Waiting...".to_owned(), 0.0),
    };

    let gauge = Gauge::default()
        .block(Block::default().title("Progress").borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Green))
        .ratio(ratio.clamp(0.0, 1.0))
        .label(label);
    frame.render_widget(gauge, layout[0]);
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for game in GameId::ALL {
        let cfg = app.config.games.get(&game);
        let vo = cfg.map(|c| c.vo_lang.as_str()).unwrap_or("en-us");
        let path = cfg
            .and_then(|c| c.install_path.as_ref())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not set".to_owned());
        lines.push(Line::from(vec![
            Span::styled(game.display_name(), Style::default().fg(Color::Yellow)),
            Span::raw(format!("  voice: {}  path: {}", vo, path)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled("Components", Style::default().fg(Color::Cyan)));
    let proton = app
        .config
        .installed_components
        .proton
        .as_deref()
        .unwrap_or("not installed");
    let jadeite = app
        .config
        .installed_components
        .jadeite
        .as_deref()
        .unwrap_or("not installed");
    lines.push(Line::raw(format!("  Proton: {}", proton)));
    lines.push(Line::raw(format!("  Jadeite: {}", jadeite)));

    let paragraph =
        Paragraph::new(lines).block(Block::default().title("Settings").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    }
}

fn format_eta(seconds: f64) -> String {
    let s = seconds as u64;
    if s >= 60 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}s", s)
    }
}
