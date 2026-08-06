use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Padding, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, View};
use crate::game::GameId;
use irmin::SophonProgress;

/// Dark translucent panel background for text readability over the image.
const PANEL_BG: Color = Color::Rgb(10, 10, 15);

/// Renders the full TUI frame based on current application state.
pub fn draw(frame: &mut Frame, app: &App) {
    // Render background image across the full terminal area
    if matches!(app.current_view, View::GameList | View::GameDetail) {
        if let Some(bg) = app.backgrounds.get(&app.selected_game()) {
            frame.render_widget(bg, frame.area());
        }
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Game tabs
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Action bar
        ])
        .split(frame.area());

    // Top: game selector tabs
    draw_game_tabs(frame, app, outer[0]);

    // Middle: main content area
    if let Some(ref msg) = app.error_message {
        let error = Paragraph::new(format!(" {}\n\n Press any key to dismiss.", msg))
            .style(Style::default().fg(Color::Red))
            .block(
                Block::default()
                    .title(" Error ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            );
        frame.render_widget(error, outer[1]);
    } else if let Some(ref msg) = app.status_message {
        let status = Paragraph::new(format!(" {}\n\n Press any key to continue.", msg))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .title(" Info ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        frame.render_widget(status, outer[1]);
    } else {
        match app.current_view {
            View::GameList | View::GameDetail => draw_main_panel(frame, app, outer[1]),
            View::Downloading => draw_downloading(frame, app, outer[1]),
            View::Settings => draw_settings(frame, app, outer[1]),
        }
    }

    // Bottom: action bar with primary button + keybinds
    draw_action_bar(frame, app, outer[2]);
}

fn draw_game_tabs(frame: &mut Frame, app: &App, area: Rect) {
    // Clear background quadrant chars, then draw over with styled block
    frame.render_widget(Clear, area);

    let titles: Vec<Line> = GameId::ALL
        .iter()
        .map(|g| {
            let color = game_color(*g);
            Line::from(Span::styled(
                format!(" {} ", g.display_name()),
                Style::default().fg(color),
            ))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(app.game_list_index)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(game_color(app.selected_game()))
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled(" | ", Style::default().fg(Color::DarkGray)))
        .block(
            Block::default()
                .title(Span::styled(
                    " elysiae ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(Color::DarkGray))
                .style(Style::default().bg(PANEL_BG)),
        );
    frame.render_widget(tabs, area);
}

fn draw_main_panel(frame: &mut Frame, app: &App, area: Rect) {
    let game = app.selected_game();
    let status = app.games.get(&game);
    let installed = status.and_then(|s| s.installed_tag.as_ref());

    // Border-only block — no padding, no bg. Background image shows through.
    // Content spacing is handled via leading spaces in text spans.
    let panel_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(Color::DarkGray));
    let content_area = panel_block.inner(area);
    frame.render_widget(panel_block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Header: game name + version
            Constraint::Min(0),    // Info area
        ])
        .split(content_area);

    // Header block: game branding
    let version_text = installed
        .map(|t| format!("v{}", t))
        .unwrap_or_else(|| "Not installed".to_owned());

    let header_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", game.display_name().to_uppercase()),
            Style::default()
                .fg(game_color(game))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", version_text),
            Style::default().fg(Color::Gray),
        )),
    ];

    let header = Paragraph::new(header_lines);
    frame.render_widget(header, layout[0]);

    // Info area: update status + actions
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    if let Some(gs) = status {
        if let Some(ref info) = gs.update_info {
            if info.update_available {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  Update available  ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "{} ({})",
                            info.remote_tag,
                            format_bytes(info.update_compressed_size)
                        ),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
                lines.push(Line::from(""));
            }
            if info.preinstall_available {
                let tag = info.preinstall_tag.as_deref().unwrap_or("unknown");
                let suffix = if info.preinstall_downloaded {
                    " [ready to apply]"
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::styled("  Preinstall  ", Style::default().fg(Color::Magenta)),
                    Span::styled(
                        format!("{}{}", tag, suffix),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
                lines.push(Line::from(""));
            }
        }
    }

    // Action hints styled like menu items
    let is_installed = installed.is_some();
    let has_update = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.update_available);
    let has_preinstall = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.preinstall_available && !i.preinstall_downloaded);

    lines.push(Line::styled(
        "  Actions",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(""));

    if !is_installed {
        lines.push(action_line('d', "Download Game", Color::Yellow));
    }
    if has_update {
        lines.push(action_line('u', "Update", Color::Green));
    }
    if is_installed {
        lines.push(action_line('l', "Launch", Color::Cyan));
        lines.push(action_line('v', "Verify Files", Color::White));
    }
    if has_preinstall {
        lines.push(action_line('p', "Preinstall", Color::Magenta));
    }

    let info = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
    frame.render_widget(info, layout[1]);
}

fn draw_downloading(frame: &mut Frame, app: &App, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Game name
            Constraint::Min(0),    // Progress
            Constraint::Length(2), // Speed/ETA
        ])
        .split(area);

    let game = app
        .download
        .as_ref()
        .map(|d| d.game_id)
        .unwrap_or(app.active_game);
    let title = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            game.display_name(),
            Style::default()
                .fg(game_color(game))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    frame.render_widget(title, layout[0]);

    let progress = app.download.as_ref().map(|d| &d.progress);
    let (label, ratio, detail) = progress_info(progress);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .gauge_style(Style::default().fg(game_color(game)).bg(Color::DarkGray))
        .ratio(ratio.clamp(0.0, 1.0))
        .label(Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, layout[1]);

    let detail_line = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(detail, Style::default().fg(Color::Gray)),
    ]));
    frame.render_widget(detail_line, layout[2]);
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(Clear, area);

    let panel_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(PANEL_BG))
        .padding(Padding::new(2, 2, 1, 1));
    let content_area = panel_block.inner(area);
    frame.render_widget(panel_block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Section header
            Constraint::Min(0),    // Content
        ])
        .split(content_area);

    let header = Paragraph::new(Line::from(Span::styled(
        "  Settings",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, layout[0]);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::styled(
        "  Games",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(""));

    for game in GameId::ALL {
        let cfg = app.config.games.get(&game);
        let vo = cfg.map(|c| c.vo_lang.as_str()).unwrap_or("en-us");
        let path = cfg
            .and_then(|c| c.install_path.as_ref())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not set".to_owned());
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<6}", game.display_name()),
                Style::default().fg(game_color(game)),
            ),
            Span::styled(format!("lang: {:<6}", vo), Style::default().fg(Color::Gray)),
            Span::styled(
                format!("path: {}", path),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Components",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(""));

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
    lines.push(Line::from(vec![
        Span::styled("  [1] Proton   ", Style::default().fg(Color::Green)),
        Span::styled(proton, Style::default().fg(Color::Gray)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  [2] Jadeite  ", Style::default().fg(Color::Magenta)),
        Span::styled(jadeite, Style::default().fg(Color::Gray)),
    ]));

    let content = Paragraph::new(lines);
    frame.render_widget(content, layout[1]);
}

fn draw_action_bar(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(Clear, area);

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(20)])
        .split(area);

    // Left: keybinds
    let keys = match app.current_view {
        View::GameList | View::GameDetail => " q quit  s settings  <-/-> switch game",
        View::Downloading => " p pause  r resume  c cancel",
        View::Settings => " esc back  1 proton  2 jadeite",
    };
    let keybinds = Paragraph::new(Line::from(Span::styled(
        keys,
        Style::default().fg(Color::DarkGray),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(PANEL_BG)),
    );
    frame.render_widget(keybinds, layout[0]);

    // Right: primary action button
    let (btn_text, btn_color) = primary_button(app);
    let button = Paragraph::new(Line::from(Span::styled(
        format!(" {} ", btn_text),
        Style::default()
            .fg(Color::Black)
            .bg(btn_color)
            .add_modifier(Modifier::BOLD),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(PANEL_BG)),
    );
    frame.render_widget(button, layout[1]);
}

fn primary_button(app: &App) -> (&'static str, Color) {
    match app.current_view {
        View::Downloading => ("Downloading...", Color::Yellow),
        View::Settings => ("Settings", Color::Cyan),
        _ => {
            let game = app.selected_game();
            let installed = app
                .games
                .get(&game)
                .and_then(|s| s.installed_tag.as_ref())
                .is_some();
            let has_update = app
                .games
                .get(&game)
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available);
            if has_update {
                ("Update", Color::Green)
            } else if installed {
                ("Launch", Color::Cyan)
            } else {
                ("Get Game", Color::Yellow)
            }
        }
    }
}

fn action_line(key: char, label: &str, color: Color) -> Line<'_> {
    Line::from(vec![
        Span::styled(
            format!("    [{}] ", key),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(label, Style::default().fg(color)),
    ])
}

fn game_color(game: GameId) -> Color {
    match game {
        GameId::Bh3 => Color::LightRed,
        GameId::Hk4e => Color::Yellow,
        GameId::Hkrpg => Color::LightCyan,
        GameId::Nap => Color::LightGreen,
    }
}

fn progress_info(progress: Option<&SophonProgress>) -> (String, f64, String) {
    match progress {
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
            let label = format!("{:.1}%", pct * 100.0);
            let detail = format!(
                "{}/s  ETA {}",
                format_bytes(*speed_bps as u64),
                format_eta(*eta_seconds)
            );
            (label, pct, detail)
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
            (
                format!("Paused {:.1}%", pct * 100.0),
                pct,
                "Paused".to_owned(),
            )
        }
        Some(SophonProgress::FetchingManifest) => (
            "Fetching...".to_owned(),
            0.0,
            "Fetching manifest".to_owned(),
        ),
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
                format!("{}/{}", checked_files, total_files),
                pct,
                "Calculating downloads".to_owned(),
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
                format!("{}/{}", assembled_files, total_files),
                pct,
                "Assembling files".to_owned(),
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
                format!("{}/{}", scanned_files, total_files),
                pct,
                format!("Verifying ({} errors)", error_count),
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
                format!("{}/{}", checked_files, total_files),
                pct,
                "Checking files".to_owned(),
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
                format!("{}/{}", applied_files, total_files),
                pct,
                "Applying preinstall".to_owned(),
            )
        }
        Some(SophonProgress::InstallingPlugins { current_plugin, .. }) => {
            (current_plugin.clone(), 0.0, "Installing plugins".to_owned())
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
            (
                format!("{:.1}%", pct * 100.0),
                pct,
                format!("Plugin: {}", name),
            )
        }
        Some(SophonProgress::Finished) => ("Done".to_owned(), 1.0, "Complete".to_owned()),
        Some(SophonProgress::Error { message }) => ("Error".to_owned(), 0.0, message.clone()),
        Some(SophonProgress::Warning { message }) => ("Warning".to_owned(), 0.0, message.clone()),
        _ => ("...".to_owned(), 0.0, "Waiting".to_owned()),
    }
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
