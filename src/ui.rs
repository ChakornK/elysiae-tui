use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Padding, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, View};
use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Dark translucent panel background for text readability over the image.
/// Approximates ~50% opacity black over a typical dark game background.
const PANEL_BG: Color = Color::Rgb(15, 15, 22);

/// Renders the full TUI frame based on current application state.
pub fn draw(frame: &mut Frame, app: &App) {
    // Render background image across the full terminal area
    if matches!(
        app.current_view,
        View::GameList | View::GameDetail | View::Downloading
    ) {
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
            View::GameList | View::GameDetail | View::Downloading => {
                draw_main_panel(frame, app, outer[1]);
                // Draw progress overlay if download is active
                if app.download.is_some() {
                    draw_progress_overlay(frame, app, outer[1]);
                }
            }
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
    let bg_img = app.backgrounds.get(&game);

    // Build content lines for the info container
    let mut info_lines: Vec<Line> = Vec::new();

    if let Some(gs) = status {
        if let Some(ref info) = gs.update_info {
            if info.update_available {
                info_lines.push(Line::from(vec![
                    Span::styled(
                        " Update available  ",
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
            }
            if info.preinstall_available {
                let tag = info.preinstall_tag.as_deref().unwrap_or("unknown");
                let suffix = if info.preinstall_downloaded {
                    " [ready]"
                } else {
                    ""
                };
                info_lines.push(Line::from(vec![
                    Span::styled(" Preinstall  ", Style::default().fg(Color::Magenta)),
                    Span::styled(
                        format!("{}{}", tag, suffix),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
            }
        }
    }

    // Action lines
    let is_installed = installed.is_some();
    let has_update = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.update_available);
    let has_preinstall = status
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.preinstall_available && !i.preinstall_downloaded);

    let mut action_lines: Vec<Line> = Vec::new();
    if !is_installed {
        action_lines.push(action_line('d', "Download Game", Color::Yellow));
    }
    if has_update {
        action_lines.push(action_line('u', "Update", Color::Green));
    }
    if is_installed {
        action_lines.push(action_line('l', "Launch", Color::Cyan));
        action_lines.push(action_line('v', "Verify Files", Color::White));
    }
    if has_preinstall {
        action_lines.push(action_line('p', "Preinstall", Color::Magenta));
    }

    // Calculate container heights (1 row padding top + bottom)
    let header_h: u16 = 4; // padding(2) + title + version
    let info_h: u16 = if info_lines.is_empty() {
        0
    } else {
        info_lines.len() as u16 + 2 // padding(2)
    };
    let actions_h: u16 = action_lines.len() as u16 + 3; // padding(2) + "Actions" title

    // Layout containers vertically with gaps
    let mut y = area.y + 1; // 1-row margin from top
    let container_width = 50u16.min(area.width.saturating_sub(2));
    let x = area.x + 1; // 1-col margin from left

    // Header container
    let header_rect = Rect::new(
        x,
        y,
        container_width,
        header_h.min(area.bottom().saturating_sub(y)),
    );
    if header_rect.height >= 2 {
        render_container(frame, header_rect, bg_img);
        let inner = shrink(header_rect, 1, 1);
        let header_lines = vec![
            Line::from(Span::styled(
                format!(" {}", game.display_name().to_uppercase()),
                Style::default()
                    .fg(game_color(game))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(
                    " {}",
                    installed
                        .map(|t| format!("v{}", t))
                        .unwrap_or_else(|| "Not installed".to_owned())
                ),
                Style::default().fg(Color::Gray),
            )),
        ];
        frame.render_widget(Paragraph::new(header_lines), inner);
        y = header_rect.bottom() + 1;
    }

    // Info container (only if there's update/preinstall info)
    if !info_lines.is_empty() && y < area.bottom().saturating_sub(1) {
        let info_rect = Rect::new(
            x,
            y,
            container_width,
            info_h.min(area.bottom().saturating_sub(y)),
        );
        if info_rect.height >= 1 {
            render_container(frame, info_rect, bg_img);
            let inner = shrink(info_rect, 1, 1);
            frame.render_widget(Paragraph::new(info_lines), inner);
            y = info_rect.bottom() + 1;
        }
    }

    // Actions container
    if !action_lines.is_empty() && y < area.bottom().saturating_sub(1) {
        let actions_rect = Rect::new(
            x,
            y,
            container_width,
            actions_h.min(area.bottom().saturating_sub(y)),
        );
        if actions_rect.height >= 1 {
            render_container(frame, actions_rect, bg_img);
            let inner = shrink(actions_rect, 1, 1);
            let actions_content = Paragraph::new(action_lines).block(
                Block::default()
                    .title(Span::styled(
                        " Actions ",
                        Style::default().fg(Color::DarkGray),
                    ))
                    .borders(Borders::NONE),
            );
            frame.render_widget(actions_content, inner);
        }
    }
}

/// Renders a dark container with 50% opacity over the background image.
/// Reads bg image colors and blends them with black.
fn render_container(frame: &mut Frame, area: Rect, bg_img: Option<&QuadrantImage>) {
    let buf = frame.buffer_mut();
    for cy in area.y..area.bottom() {
        for cx in area.x..area.right() {
            let (r, g, b) = if let Some(img) = bg_img {
                // Read original color from the pre-rendered background image
                let idx = (cy as usize) * (img.width as usize) + (cx as usize);
                if idx < img.cells.len() {
                    match img.cells[idx].bg {
                        Color::Rgb(r, g, b) => (r, g, b),
                        _ => (0, 0, 0),
                    }
                } else {
                    (0, 0, 0)
                }
            } else {
                (0, 0, 0)
            };
            // 50% blend with black
            let blended = Color::Rgb(r / 2, g / 2, b / 2);
            let cell = &mut buf[(cx, cy)];
            cell.set_char(' ');
            cell.set_bg(blended);
            cell.set_fg(Color::Reset);
        }
    }
}

/// Shrinks a rect by the given horizontal and vertical margins (for inside a border).
fn shrink(r: Rect, h: u16, v: u16) -> Rect {
    Rect::new(
        r.x + h,
        r.y + v,
        r.width.saturating_sub(h * 2),
        r.height.saturating_sub(v * 2),
    )
}

/// Draws compact progress bars in the bottom-left corner of the main area.
fn draw_progress_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let Some(dl) = &app.download else { return };
    let bg_img = app.backgrounds.get(&dl.game_id);
    let game = dl.game_id;

    // Calculate how many rows we need
    let mut rows: u16 = 0;
    if dl.status_label.is_some() {
        rows += 1;
    }
    if dl.download_progress.is_some() {
        rows += 1;
    }
    if dl.verify_progress.is_some() {
        rows += 1;
    }
    if rows == 0 {
        return;
    }
    rows += 2; // vertical padding

    let overlay_width = 55u16.min(area.width.saturating_sub(2));
    let overlay_rect = Rect::new(
        area.x + 1,
        area.bottom().saturating_sub(rows + 1),
        overlay_width,
        rows,
    );

    if overlay_rect.height == 0 || overlay_rect.y < area.y {
        return;
    }

    render_container(frame, overlay_rect, bg_img);
    let inner = shrink(overlay_rect, 2, 1);
    let mut y = inner.y;

    // Status label (fetching manifest, calculating, plugins, etc.)
    if let Some(ref label) = dl.status_label {
        let line = Line::from(Span::styled(
            label.as_str(),
            Style::default().fg(Color::Gray),
        ));
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Download bar
    if let Some(ref dp) = dl.download_progress {
        let pct = if dp.total_bytes > 0 {
            dp.downloaded_bytes as f64 / dp.total_bytes as f64
        } else {
            0.0
        };
        let label = if dp.speed_bps > 0.0 {
            format!(
                " Downloading {:.1}%  {}/s  ETA {}",
                pct * 100.0,
                format_bytes(dp.speed_bps as u64),
                format_eta(dp.eta_seconds)
            )
        } else {
            format!(
                " Downloading {:.1}%  {}/{}",
                pct * 100.0,
                format_bytes(dp.downloaded_bytes),
                format_bytes(dp.total_bytes)
            )
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(game_color(game)).bg(Color::DarkGray))
            .ratio(pct.clamp(0.0, 1.0))
            .label(Span::styled(label, Style::default().fg(Color::White)));
        frame.render_widget(gauge, Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Verify/assemble bar
    if let Some(ref vp) = dl.verify_progress {
        let pct = if vp.total > 0 {
            vp.done as f64 / vp.total as f64
        } else {
            0.0
        };
        let label = format!(
            " {} {}/{}  {:.1}%",
            vp.label,
            vp.done,
            vp.total,
            pct * 100.0
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
            .ratio(pct.clamp(0.0, 1.0))
            .label(Span::styled(label, Style::default().fg(Color::White)));
        frame.render_widget(gauge, Rect::new(inner.x, y, inner.width, 1));
    }
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
    let keys = if app.download.is_some() {
        " q quit  p pause  r resume  c cancel  <-/-> switch game"
    } else {
        match app.current_view {
            View::GameList | View::GameDetail | View::Downloading => {
                " q quit  s settings  <-/-> switch game"
            }
            View::Settings => " esc back  1 proton  2 jadeite",
        }
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
    if app.download.is_some() {
        return ("Downloading...", Color::Yellow);
    }
    match app.current_view {
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
        Span::styled(format!(" [{}] ", key), Style::default().fg(Color::DarkGray)),
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
