use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Padding, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, View};
use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Application color palette — fixed RGB values independent of terminal theme.
mod palette {
    use ratatui::style::Color;

    pub const PANEL_BG: Color = Color::Rgb(15, 15, 22);
    pub const TEXT: Color = Color::Rgb(220, 220, 230);
    pub const TEXT_MUTED: Color = Color::Rgb(170, 170, 170);
    pub const BORDER: Color = Color::Rgb(70, 70, 90);
    pub const ERROR: Color = Color::Rgb(240, 80, 80);
    pub const WARNING: Color = Color::Rgb(240, 200, 60);
    pub const SUCCESS: Color = Color::Rgb(80, 220, 120);
    pub const ACCENT: Color = Color::Rgb(100, 200, 240);
    pub const MAGENTA: Color = Color::Rgb(200, 140, 220);
    pub const BAR_BG: Color = Color::Rgb(50, 50, 65);
    pub const BLACK: Color = Color::Rgb(10, 10, 15);
    pub const CONTAINER_BG: Color = Color::Rgb(37, 37, 37);

    // Per-game brand colors
    pub const GAME_BH3: Color = Color::Rgb(240, 120, 120);
    pub const GAME_HK4E: Color = Color::Rgb(240, 200, 60);
    pub const GAME_HKRPG: Color = Color::Rgb(120, 220, 240);
    pub const GAME_NAP: Color = Color::Rgb(120, 240, 140);
}

use palette::*;

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
            Constraint::Length(4), // Game tabs (1 gap + 3 container)
            Constraint::Min(0),    // Main content
            Constraint::Length(4), // Action bar
        ])
        .split(frame.area());

    // Inset the top and bottom bars for floating effect
    let tab_area = Rect::new(
        outer[0].x + 2,
        outer[0].y + 1,
        outer[0].width.saturating_sub(4),
        outer[0].height.saturating_sub(1),
    );
    let bar_area = Rect::new(
        outer[2].x + 2,
        outer[2].y,
        outer[2].width.saturating_sub(4),
        outer[2].height.saturating_sub(1),
    );

    // Top: game selector tabs
    draw_game_tabs(frame, app, tab_area);

    // Middle: main content area
    if let Some(ref msg) = app.error_message {
        let error = Paragraph::new(format!(" {}\n\n Press any key to dismiss.", msg))
            .style(Style::default().fg(ERROR))
            .block(
                Block::default()
                    .title(" Error ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ERROR)),
            );
        frame.render_widget(error, outer[1]);
    } else if let Some(ref msg) = app.status_message {
        let status = Paragraph::new(format!(" {}\n\n Press any key to continue.", msg))
            .style(Style::default().fg(WARNING))
            .block(
                Block::default()
                    .title(" Info ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(WARNING)),
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
    draw_action_bar(frame, app, bar_area);
}

fn draw_game_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let bg_img = app.backgrounds.get(&app.selected_game());
    render_container(frame, area, bg_img);
    let inner = shrink(area, 2, 1);

    // Build single line: "elysiae | [1] bh3 | [2] hk4e | ..."
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(
        "elysiae",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled("  ", Style::default()));

    for (i, g) in GameId::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(TEXT_MUTED)));
        }
        let label = format!("[{}] {} ", i + 1, g.display_name());
        if i == app.game_list_index {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(BLACK)
                    .bg(game_color(*g))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label, Style::default().fg(game_color(*g))));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
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
                        Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "{} ({})",
                            info.remote_tag,
                            format_bytes(info.update_compressed_size)
                        ),
                        Style::default().fg(TEXT_MUTED),
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
                    Span::styled(" Preinstall  ", Style::default().fg(MAGENTA)),
                    Span::styled(
                        format!("{}{}", tag, suffix),
                        Style::default().fg(TEXT_MUTED),
                    ),
                ]));
            }
        }
    }

    // Calculate container heights (1 row padding top + bottom)
    let header_h: u16 = 4; // padding(2) + title + version
    let info_h: u16 = if info_lines.is_empty() {
        0
    } else {
        info_lines.len() as u16 + 2 // padding(2)
    };

    // Layout containers vertically with gaps
    let mut y = area.y + 1; // 1-row margin from top
    let container_width = 50u16.min(area.width.saturating_sub(4));
    let x = area.x + 2; // 2-col margin from left

    // Header container
    let header_rect = Rect::new(
        x,
        y,
        container_width,
        header_h.min(area.bottom().saturating_sub(y)),
    );
    if header_rect.height >= 2 {
        render_container(frame, header_rect, bg_img);
        let inner = shrink(header_rect, 2, 1);
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
                Style::default().fg(TEXT_MUTED),
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
            let inner = shrink(info_rect, 2, 1);
            frame.render_widget(Paragraph::new(info_lines), inner);
            y = info_rect.bottom() + 1;
        }
    }
}

/// Renders a dark gray container over the background image.
fn render_container(frame: &mut Frame, area: Rect, _bg_img: Option<&QuadrantImage>) {
    let buf = frame.buffer_mut();
    for cy in area.y..area.bottom() {
        for cx in area.x..area.right() {
            let cell = &mut buf[(cx, cy)];
            cell.set_char(' ');
            cell.set_bg(CONTAINER_BG);
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
        if rows > 0 {
            rows += 1;
        }
        rows += 1;
    }
    if dl.assemble_progress.is_some() {
        if rows > 0 {
            rows += 1;
        }
        rows += 1;
    }
    if dl.check_progress.is_some() {
        if rows > 0 {
            rows += 1;
        }
        rows += 1;
    }
    // Speed/ETA always at bottom when downloading
    if dl.download_progress.is_some() {
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

    // Status label (fetching manifest, plugins, etc.)
    if let Some(ref label) = dl.status_label {
        let line = Line::from(Span::styled(
            label.as_str(),
            Style::default().fg(TEXT_MUTED),
        ));
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Download bar: "Downloading - X.XX GB/Y.YY GB (Z.ZZ%)"
    if let Some(ref dp) = dl.download_progress {
        if y > inner.y {
            y += 1;
        }

        let pct = if dp.total_bytes > 0 {
            dp.downloaded_bytes as f64 / dp.total_bytes as f64
        } else {
            0.0
        };
        let bar_label = format!(
            "Downloading - {}/{} ({:.2}%)",
            format_bytes(dp.downloaded_bytes),
            format_bytes(dp.total_bytes),
            pct * 100.0
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(game_color(game)).bg(BAR_BG))
            .ratio(pct.clamp(0.0, 1.0))
            .label(Span::styled(bar_label, Style::default().fg(TEXT)));
        frame.render_widget(gauge, Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Assemble bar: "Assembled - X/Y (Z.ZZ%)"
    if let Some(ref ap) = dl.assemble_progress {
        if y > inner.y {
            y += 1;
        }

        let pct = if ap.total > 0 {
            ap.done as f64 / ap.total as f64
        } else {
            0.0
        };
        let label = format!(
            "{} - {}/{} ({:.2}%)",
            ap.label,
            ap.done,
            ap.total,
            pct * 100.0
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(MAGENTA).bg(BAR_BG))
            .ratio(pct.clamp(0.0, 1.0))
            .label(Span::styled(label, Style::default().fg(TEXT)));
        frame.render_widget(gauge, Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Check/verify bar: "Checked - X/Y (Z.ZZ%)"
    if let Some(ref cp) = dl.check_progress {
        if y > inner.y {
            y += 1;
        }

        let pct = if cp.total > 0 {
            cp.done as f64 / cp.total as f64
        } else {
            0.0
        };
        let label = format!(
            "{} - {}/{} ({:.2}%)",
            cp.label,
            cp.done,
            cp.total,
            pct * 100.0
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(ACCENT).bg(BAR_BG))
            .ratio(pct.clamp(0.0, 1.0))
            .label(Span::styled(label, Style::default().fg(TEXT)));
        frame.render_widget(gauge, Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    if let Some(ref dp) = dl.download_progress {
        let w = inner.width as usize;
        let line = if dp.speed_bps > 0.0 {
            let speed = format!("{}/s", format_bytes(dp.speed_bps as u64));
            let eta = format!("ETA {}", format_eta_long(dp.eta_seconds));
            let pad = w.saturating_sub(speed.len() + eta.len());
            Line::from(vec![
                Span::styled(speed, Style::default().fg(TEXT_MUTED)),
                Span::raw(" ".repeat(pad)),
                Span::styled(eta, Style::default().fg(TEXT_MUTED)),
            ])
        } else if dp.downloaded_bytes > 0 {
            Line::from(Span::styled("Paused", Style::default().fg(TEXT_MUTED)))
        } else {
            Line::from(Span::styled("Starting...", Style::default().fg(TEXT_MUTED)))
        };
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
    }
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(Clear, area);

    let panel_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::ROUNDED)
        .border_style(Style::default().fg(BORDER))
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
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, layout[0]);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::styled("  Games", Style::default().fg(TEXT_MUTED)));
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
            Span::styled(format!("lang: {:<6}", vo), Style::default().fg(TEXT_MUTED)),
            Span::styled(format!("path: {}", path), Style::default().fg(TEXT_MUTED)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Components",
        Style::default().fg(TEXT_MUTED),
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
        Span::styled("  [1] Proton   ", Style::default().fg(SUCCESS)),
        Span::styled(proton, Style::default().fg(TEXT_MUTED)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  [2] Jadeite  ", Style::default().fg(MAGENTA)),
        Span::styled(jadeite, Style::default().fg(TEXT_MUTED)),
    ]));

    let content = Paragraph::new(lines);
    frame.render_widget(content, layout[1]);
}

fn draw_action_bar(frame: &mut Frame, app: &App, area: Rect) {
    let bg_img = app.backgrounds.get(&app.selected_game());

    // Right island: button with 2 col 1 row padding
    let (btn_text, btn_color, btn_key) = primary_button(app);
    let btn_label = format!("[{}] {}", btn_key, btn_text);
    let btn_content_w = btn_label.len() as u16;
    let btn_island_w = btn_content_w + 4; // + 2 col padding each side
    let btn_island_h = 3u16; // 1 row padding top + content + 1 row padding bottom

    let btn_rect = Rect::new(
        area.right().saturating_sub(btn_island_w),
        area.y,
        btn_island_w.min(area.width),
        btn_island_h.min(area.height),
    );

    // Left island: keybinds
    let keys = if app.download.is_some() {
        "[q] quit  [p] pause/resume  [c] cancel  [←/→] switch game"
    } else {
        match app.current_view {
            View::GameList | View::GameDetail | View::Downloading => {
                "[q] quit  [s] settings  [←/→] switch game"
            }
            View::Settings => "[esc] back  [1] proton  [2] jadeite",
        }
    };
    let keys_display_w = keys.chars().count() as u16;
    let keys_w = (keys_display_w + 4).min(area.width.saturating_sub(btn_island_w + 2));
    let keys_rect = Rect::new(area.x, area.y, keys_w, btn_island_h.min(area.height));

    // Render left island
    render_container(frame, keys_rect, bg_img);
    let keys_inner = shrink(keys_rect, 2, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            keys,
            Style::default().fg(TEXT_MUTED),
        ))),
        keys_inner,
    );

    // Render right button (solid color, no container)
    let buf = frame.buffer_mut();
    for cy in btn_rect.y..btn_rect.bottom() {
        for cx in btn_rect.x..btn_rect.right() {
            let cell = &mut buf[(cx, cy)];
            cell.set_char(' ');
            cell.set_bg(btn_color);
            cell.set_fg(BLACK);
        }
    }
    let btn_inner = shrink(btn_rect, 2, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            btn_label,
            Style::default()
                .fg(BLACK)
                .bg(btn_color)
                .add_modifier(Modifier::BOLD),
        ))),
        btn_inner,
    );
}

fn primary_button(app: &App) -> (&'static str, Color, &'static str) {
    if app.download.is_some() {
        return ("Downloading...", WARNING, "p");
    }
    match app.current_view {
        View::Settings => ("Settings", ACCENT, "s"),
        _ => {
            let game = app.selected_game();
            let status = app.games.get(&game);
            let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
            let has_update = status
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available);
            let has_resume = status.is_some_and(|s| s.has_resume);

            if has_update {
                ("Update", SUCCESS, "⏎")
            } else if has_resume {
                ("Resume", WARNING, "⏎")
            } else if installed {
                ("Launch", ACCENT, "⏎")
            } else {
                ("Get Game", WARNING, "⏎")
            }
        }
    }
}

fn game_color(game: GameId) -> Color {
    match game {
        GameId::Bh3 => GAME_BH3,
        GameId::Hk4e => GAME_HK4E,
        GameId::Hkrpg => GAME_HKRPG,
        GameId::Nap => GAME_NAP,
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

fn format_eta_long(seconds: f64) -> String {
    let s = seconds as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{} h {} m {} s", h, m, sec)
    } else if m > 0 {
        format!("{} m {} s", m, sec)
    } else {
        format!("{} s", sec)
    }
}
