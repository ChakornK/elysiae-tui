use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, View};
use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Application color palette — fixed RGB values independent of terminal theme.
mod palette {
    use ratatui::style::Color;

    pub const TEXT: Color = Color::Rgb(220, 220, 230);
    pub const TEXT_MUTED: Color = Color::Rgb(170, 170, 170);
    pub const ERROR: Color = Color::Rgb(240, 80, 80);
    pub const WARNING: Color = Color::Rgb(240, 200, 60);
    pub const SUCCESS: Color = Color::Rgb(80, 220, 120);
    pub const ACCENT: Color = Color::Rgb(100, 200, 240);
    pub const MAGENTA: Color = Color::Rgb(200, 140, 220);
    pub const BAR_BG: Color = Color::Rgb(50, 50, 65);
    pub const BLACK: Color = Color::Rgb(10, 10, 15);
    pub const CONTAINER_BG: Color = Color::Rgb(26, 26, 26);
    pub const SECONDARY_BG: Color = Color::Rgb(18, 18, 18);

    pub const GAME_BH3: Color = Color::Rgb(240, 120, 120);
    pub const GAME_HK4E: Color = Color::Rgb(240, 200, 60);
    pub const GAME_HKRPG: Color = Color::Rgb(120, 220, 240);
    pub const GAME_NAP: Color = Color::Rgb(120, 240, 140);
}

use palette::*;

// Layout constants
const EDGE_PAD_H: u16 = 2;
const EDGE_PAD_V: u16 = 1;
const MAIN_PANEL_MAX_WIDTH: u16 = 50;
const PROGRESS_MAX_WIDTH: u16 = 55;
const TAB_BAR_HEIGHT: u16 = 4;
const ACTION_BAR_HEIGHT: u16 = 4;
const MIN_TERMINAL_WIDTH: u16 = 40;
const MIN_TERMINAL_HEIGHT: u16 = 10;

/// Renders the full TUI frame based on current application state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Bail out if terminal is too small to render
    if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
        let msg = "Terminal too small";
        let x = area.width.saturating_sub(msg.len() as u16) / 2;
        let y = area.height / 2;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(TEXT_MUTED),
            ))),
            Rect::new(x, y, msg.len() as u16, 1),
        );
        return;
    }

    // Render background image across the full terminal area
    if let Some(bg) = app.backgrounds.get(&app.selected_game()) {
        frame.render_widget(bg, frame.area());
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TAB_BAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(ACTION_BAR_HEIGHT),
        ])
        .split(frame.area());

    // Inset top and bottom bars for floating effect
    let tab_area = Rect::new(
        outer[0].x + EDGE_PAD_H,
        outer[0].y + EDGE_PAD_V,
        outer[0].width.saturating_sub(EDGE_PAD_H * 2),
        outer[0].height.saturating_sub(EDGE_PAD_V),
    );
    let bar_area = Rect::new(
        outer[2].x + EDGE_PAD_H,
        outer[2].y,
        outer[2].width.saturating_sub(EDGE_PAD_H * 2),
        outer[2].height.saturating_sub(EDGE_PAD_V),
    );

    // Top: game selector tabs
    draw_game_tabs(frame, app, tab_area);

    // Middle: main content area — always render the main panel
    match app.current_view {
        View::GameList | View::Settings => {
            draw_main_panel(frame, app, outer[1]);
            if app.download.is_some() {
                draw_progress_overlay(frame, app, outer[1]);
            } else if app.game_running
                && !app.launch_log.is_empty()
                && app.launch_log_game == Some(app.selected_game())
            {
                draw_launch_log(frame, app, outer[1]);
            }
        }
    }

    // Bottom: action bar with primary button + keybinds
    draw_action_bar(frame, app, bar_area);

    // Modal overlays on top of everything
    if app.current_view == View::Settings {
        darken_full_window(frame);
        draw_settings(frame, app, outer[1]);
    }

    if let Some(ref msg) = app.error_message {
        darken_full_window(frame);
        draw_modal(frame, outer[1], "Error", msg, ERROR);
    } else if let Some(ref msg) = app.status_message {
        darken_full_window(frame);
        draw_modal(frame, outer[1], "Info", msg, WARNING);
    }

    if app.show_help {
        darken_full_window(frame);
        draw_help_overlay(frame, area);
    }

    if app.show_cancel_confirm {
        darken_full_window(frame);
        draw_modal(
            frame,
            outer[1],
            "Confirm",
            "Cancel download? (y/n)",
            WARNING,
        );
    }
}

/// Darkens the entire window with 70% #1a1a1a overlay.
fn darken_full_window(frame: &mut Frame) {
    let full = frame.area();
    let buf = frame.buffer_mut();
    for cy in full.y..full.bottom() {
        for cx in full.x..full.right() {
            let cell = &mut buf[(cx, cy)];
            if let Color::Rgb(r, g, b) = cell.bg {
                cell.set_bg(Color::Rgb(
                    ((r as u16 * 3 + 26 * 7) / 10) as u8,
                    ((g as u16 * 3 + 26 * 7) / 10) as u8,
                    ((b as u16 * 3 + 26 * 7) / 10) as u8,
                ));
            }
            if let Color::Rgb(r, g, b) = cell.fg {
                cell.set_fg(Color::Rgb(
                    ((r as u16 * 3 + 26 * 7) / 10) as u8,
                    ((g as u16 * 3 + 26 * 7) / 10) as u8,
                    ((b as u16 * 3 + 26 * 7) / 10) as u8,
                ));
            }
        }
    }
}

/// Draws a centered modal with a title, message, and [esc] Close hint.
fn draw_modal(frame: &mut Frame, area: Rect, title: &str, message: &str, color: Color) {
    let bg_img: Option<&QuadrantImage> = None;
    let msg_lines: Vec<&str> = message.lines().collect();
    let msg_w = msg_lines
        .iter()
        .map(|l| UnicodeWidthStr::width(*l))
        .max()
        .unwrap_or(20) as u16;
    let overlay_w = (msg_w + 8).max(30).min(area.width.saturating_sub(4));
    let overlay_h = (msg_lines.len() as u16 + 6).min(area.height.saturating_sub(2));

    let overlay_rect = Rect::new(
        area.x + (area.width.saturating_sub(overlay_w)) / 2,
        area.y + (area.height.saturating_sub(overlay_h)) / 2,
        overlay_w,
        overlay_h,
    );

    render_container(frame, overlay_rect, bg_img);
    let inner = shrink(overlay_rect, 2, 1);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for l in &msg_lines {
        lines.push(Line::from(Span::styled(*l, Style::default().fg(TEXT))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[esc] Close",
        Style::default().fg(TEXT_MUTED),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws a centered help overlay listing key bindings.
fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "Key Bindings",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "─────────────",
            Style::default().fg(TEXT_MUTED),
        )),
        Line::from(Span::styled("q        quit", Style::default().fg(TEXT))),
        Line::from(Span::styled(
            "←/→/Tab  switch game",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled(
            "1-4      select game",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled(
            "Enter    download/launch",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled("v        verify", Style::default().fg(TEXT))),
        Line::from(Span::styled(
            "p        preinstall",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled(
            "a        apply preinstall",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled("s        settings", Style::default().fg(TEXT))),
        Line::from(Span::styled(
            "c        cancel download",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled(
            "?        this help",
            Style::default().fg(TEXT),
        )),
        Line::from(Span::styled("Esc      back", Style::default().fg(TEXT))),
        Line::from(""),
        Line::from(Span::styled(
            "[any key] Close",
            Style::default().fg(TEXT_MUTED),
        )),
    ];

    let overlay_w = 34u16.min(area.width.saturating_sub(4));
    let overlay_h = (help_text.len() as u16 + 2).min(area.height.saturating_sub(2));
    let overlay_rect = Rect::new(
        area.x + (area.width.saturating_sub(overlay_w)) / 2,
        area.y + (area.height.saturating_sub(overlay_h)) / 2,
        overlay_w,
        overlay_h,
    );

    render_container(frame, overlay_rect, None);
    let inner = shrink(overlay_rect, 2, 1);
    frame.render_widget(Paragraph::new(help_text), inner);
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

    if let Some(gs) = status
        && let Some(ref info) = gs.update_info
    {
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

    // Calculate container heights (1 row padding top + bottom)
    let header_h: u16 = 4; // padding(2) + title + version
    let info_h: u16 = if info_lines.is_empty() {
        0
    } else {
        info_lines.len() as u16 + 2 // padding(2)
    };

    // Layout containers vertically with gaps
    let mut y = area.y + 1;
    let container_width = MAIN_PANEL_MAX_WIDTH.min(area.width.saturating_sub(EDGE_PAD_H * 2));
    let x = area.x + EDGE_PAD_H;

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

        let has_resume = status.is_some_and(|s| s.has_resume);
        let has_update = status
            .and_then(|s| s.update_info.as_ref())
            .is_some_and(|i| i.update_available);
        let has_preinstall = status
            .and_then(|s| s.update_info.as_ref())
            .is_some_and(|i| i.preinstall_available);

        let state_text = if let Some(tag) = installed {
            if has_update {
                format!("v{} - update available", tag)
            } else if has_preinstall {
                format!("v{} - preinstall available", tag)
            } else {
                format!("v{}", tag)
            }
        } else if has_resume {
            "Partially downloaded".to_owned()
        } else {
            "Not installed".to_owned()
        };

        let header_lines = vec![
            Line::from(Span::styled(
                format!(" {}", game.display_name().to_uppercase()),
                Style::default()
                    .fg(game_color(game))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", state_text),
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
        }
    }
}

/// Renders a container with #1a1a1a at 70% opacity over the existing buffer content.
fn render_container(frame: &mut Frame, area: Rect, _bg_img: Option<&QuadrantImage>) {
    let buf = frame.buffer_mut();
    for cy in area.y..area.bottom() {
        for cx in area.x..area.right() {
            let cell = &mut buf[(cx, cy)];
            if let Color::Rgb(r, g, b) = cell.bg {
                cell.set_bg(Color::Rgb(
                    ((r as u16 * 3 + 26 * 7) / 10) as u8,
                    ((g as u16 * 3 + 26 * 7) / 10) as u8,
                    ((b as u16 * 3 + 26 * 7) / 10) as u8,
                ));
            } else {
                cell.set_bg(CONTAINER_BG);
            }
            cell.set_char(' ');
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

/// Renders a progress bar with dark text on the filled area and light text on unfilled.
fn render_progress_bar(frame: &mut Frame, area: Rect, ratio: f64, label: &str, fill_color: Color) {
    let width = area.width as usize;
    let filled = ((width as f64) * ratio.clamp(0.0, 1.0)).round() as u16;

    // Center the label
    let label_chars: Vec<char> = label.chars().collect();
    let label_len = UnicodeWidthStr::width(label).min(width);
    let label_start = (width.saturating_sub(label_len)) / 2;

    let buf = frame.buffer_mut();
    for x in 0..area.width {
        let abs_x = area.x + x;
        let cell = &mut buf[(abs_x, area.y)];
        let in_filled = x < filled;

        // Determine which label char (if any) is at this position
        let pos = x as usize;
        let ch = if pos >= label_start && pos < label_start + label_len {
            label_chars[pos - label_start]
        } else {
            ' '
        };

        cell.set_char(ch);
        if in_filled {
            cell.set_bg(fill_color);
            cell.set_fg(SECONDARY_BG);
        } else {
            cell.set_bg(BAR_BG);
            cell.set_fg(TEXT);
        }
    }
}

/// Draws compact progress bars in the bottom-left corner of the main area.
fn draw_progress_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let Some(dl) = &app.download else { return };
    let bg_img = app.backgrounds.get(&dl.game_id);
    let game = dl.game_id;

    // Calculate how many rows we need
    let mut rows: u16 = 1; // "Installing <game>" header
    if dl.status_label.is_some() {
        rows += 2; // gap + label
    }
    if dl.download_progress.is_some() {
        rows += 2; // gap + bar
    }
    if dl.assemble_progress.is_some() {
        rows += 2; // gap + bar
    }
    if dl.check_progress.is_some() {
        rows += 2; // gap + bar
    }
    // Speed/ETA at bottom with gap
    if dl.download_progress.is_some() {
        rows += 2; // gap + line
    }
    rows += 2; // vertical padding

    let overlay_width = PROGRESS_MAX_WIDTH.min(area.width.saturating_sub(EDGE_PAD_H * 2));
    let overlay_rect = Rect::new(
        area.x + EDGE_PAD_H,
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

    // Header: "Installing <game name>"
    let header_text = dl
        .header_override
        .clone()
        .unwrap_or_else(|| format!("Installing {}", game.display_name()));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            header_text,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;

    // Status label (fetching manifest, extracting, etc.)
    if let Some(ref label) = dl.status_label {
        y += 1; // gap between header and status
        let line = Line::from(Span::styled(
            label.as_str(),
            Style::default().fg(TEXT_MUTED),
        ));
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // Download bar
    if let Some(ref dp) = dl.download_progress {
        y += 1; // gap

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
        render_progress_bar(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            pct,
            &bar_label,
            game_color(game),
        );
        y += 1;
    }

    // Assemble bar
    if let Some(ref ap) = dl.assemble_progress {
        y += 1; // gap

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
        render_progress_bar(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            pct,
            &label,
            MAGENTA,
        );
        y += 1;
    }

    // Check/verify bar
    if let Some(ref cp) = dl.check_progress {
        y += 1; // gap

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
        render_progress_bar(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            pct,
            &label,
            ACCENT,
        );
        y += 1;
    }

    // Speed/ETA at bottom with gap
    if let Some(ref dp) = dl.download_progress {
        y += 1; // gap
        let w = inner.width as usize;
        let line = if dl.paused {
            Line::from(Span::styled("Paused", Style::default().fg(TEXT_MUTED)))
        } else if dp.speed_bps > 0.0 {
            let speed = format!("{}/s", format_bytes(dp.speed_bps as u64));
            let eta = format!("ETA {}", format_eta_long(dp.eta_seconds));
            let pad = w.saturating_sub(speed.len() + eta.len());
            Line::from(vec![
                Span::styled(speed, Style::default().fg(TEXT_MUTED)),
                Span::raw(" ".repeat(pad)),
                Span::styled(eta, Style::default().fg(TEXT_MUTED)),
            ])
        } else if dp.downloaded_bytes > 0 && dp.total_bytes > 0 {
            let remaining = dp.total_bytes - dp.downloaded_bytes;
            Line::from(Span::styled(
                format!("{} remaining", format_bytes(remaining)),
                Style::default().fg(TEXT_MUTED),
            ))
        } else {
            Line::from(Span::styled(
                "Downloading...",
                Style::default().fg(TEXT_MUTED),
            ))
        };
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
    }
}

/// Renders game launch log in the bottom-left (same position as progress overlay).
fn draw_launch_log(frame: &mut Frame, app: &App, area: Rect) {
    let bg_img = app.backgrounds.get(&app.selected_game());
    let visible_lines = 8u16;
    let rows = visible_lines + 3; // header + padding

    let overlay_width = PROGRESS_MAX_WIDTH.min(area.width.saturating_sub(EDGE_PAD_H * 2));
    let overlay_rect = Rect::new(
        area.x + EDGE_PAD_H,
        area.bottom().saturating_sub(rows + 1),
        overlay_width,
        rows,
    );

    if overlay_rect.height == 0 || overlay_rect.y < area.y {
        return;
    }

    render_container(frame, overlay_rect, bg_img);
    let inner = shrink(overlay_rect, 2, 1);

    // Header
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Game Log",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    // Log lines (scrolled via contiguous slice for safe access)
    let log_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        visible_lines.min(inner.height.saturating_sub(2)),
    );
    let total = app.launch_log.len();
    let start = app.launch_log_scroll.min(total);
    let end = (start + log_area.height as usize).min(total);
    let (front, back) = app.launch_log.as_slices();
    let visible_log: Vec<Line> = (start..end)
        .map(|i| {
            let l = if i < front.len() {
                &front[i]
            } else {
                &back[i - front.len()]
            };
            let truncated: String = l.chars().take(log_area.width as usize).collect();
            Line::from(Span::styled(truncated, Style::default().fg(TEXT_MUTED)))
        })
        .collect();

    frame.render_widget(Paragraph::new(visible_log), log_area);
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    let bg_img = app.backgrounds.get(&app.selected_game());

    // Center the overlay box
    let overlay_w = 60u16.min(area.width.saturating_sub(4));
    let overlay_h = 16u16.min(area.height.saturating_sub(2));
    let overlay_rect = Rect::new(
        area.x + (area.width.saturating_sub(overlay_w)) / 2,
        area.y + (area.height.saturating_sub(overlay_h)) / 2,
        overlay_w,
        overlay_h,
    );

    render_container(frame, overlay_rect, bg_img);
    let inner = shrink(overlay_rect, 2, 1);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Section header
            Constraint::Min(0),    // Content
        ])
        .split(inner);

    let header = Paragraph::new(Line::from(Span::styled(
        "Settings",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, layout[0]);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::styled("Games", Style::default().fg(TEXT_MUTED)));
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
                format!("{:<6}", game.display_name()),
                Style::default().fg(game_color(game)),
            ),
            Span::styled(format!("lang: {:<6}", vo), Style::default().fg(TEXT_MUTED)),
            Span::styled(format!("path: {}", path), Style::default().fg(TEXT_MUTED)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled("Components", Style::default().fg(TEXT_MUTED)));
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
        Span::styled("Proton   ", Style::default().fg(SUCCESS)),
        Span::styled(proton, Style::default().fg(TEXT_MUTED)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Jadeite  ", Style::default().fg(MAGENTA)),
        Span::styled(jadeite, Style::default().fg(TEXT_MUTED)),
    ]));

    let content = Paragraph::new(lines);
    frame.render_widget(content, layout[1]);
}

fn draw_action_bar(frame: &mut Frame, app: &App, area: Rect) {
    let bg_img = app.backgrounds.get(&app.selected_game());

    // Right island: primary action button (always WARNING color)
    let (btn_text, btn_key) = primary_button(app);
    let btn_label = format!("[{}] {}", btn_key, btn_text);
    let btn_content_w = UnicodeWidthStr::width(btn_label.as_str()) as u16;
    let btn_island_w = btn_content_w + 4;
    let btn_island_h = 3u16;

    // Disabled when a download is active and this button can't act
    let btn_disabled = if let Some(ref dl) = app.download {
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
        // Disabled if: downloading another game (and not launchable), OR
        // waiting for components before launch (launch_on_complete)
        (dl.game_id != game && !(installed && !has_update)) || dl.launch_on_complete
    } else {
        app.game_running && app.launch_log_game == Some(app.selected_game())
    };

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
            View::GameList => "[q] quit  [s] settings  [←/→] switch game",
            View::Settings => "[esc] back",
        }
    };
    let keys_display_w = UnicodeWidthStr::width(keys) as u16;
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

    // Check if preinstall button should show
    let has_preinstall = app
        .games
        .get(&app.selected_game())
        .and_then(|s| s.update_info.as_ref())
        .is_some_and(|i| i.preinstall_available && !i.preinstall_downloaded);

    // Preinstall button (secondary, to the left of primary with 2 col gap)
    if has_preinstall && app.download.is_none() {
        let pre_label = "[p] Preinstall";
        let pre_content_w = pre_label.len() as u16;
        let pre_island_w = pre_content_w + 4;
        let pre_rect = Rect::new(
            btn_rect.x.saturating_sub(pre_island_w + 2),
            area.y,
            pre_island_w.min(area.width),
            btn_island_h.min(area.height),
        );
        let buf = frame.buffer_mut();
        for cy in pre_rect.y..pre_rect.bottom() {
            for cx in pre_rect.x..pre_rect.right() {
                let cell = &mut buf[(cx, cy)];
                cell.set_char(' ');
                cell.set_bg(SECONDARY_BG);
                cell.set_fg(Color::Reset);
            }
        }
        let pre_inner = shrink(pre_rect, 2, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pre_label,
                Style::default()
                    .fg(WARNING)
                    .bg(SECONDARY_BG)
                    .add_modifier(Modifier::BOLD),
            ))),
            pre_inner,
        );
    }

    // Render right button (solid WARNING background)
    let btn_bg = if btn_disabled {
        // 30% brightness of WARNING
        if let Color::Rgb(r, g, b) = WARNING {
            Color::Rgb(
                ((r as u16 * 3) / 10) as u8,
                ((g as u16 * 3) / 10) as u8,
                ((b as u16 * 3) / 10) as u8,
            )
        } else {
            WARNING
        }
    } else {
        WARNING
    };
    let btn_fg = if btn_disabled { TEXT_MUTED } else { BLACK };

    let buf = frame.buffer_mut();
    for cy in btn_rect.y..btn_rect.bottom() {
        for cx in btn_rect.x..btn_rect.right() {
            let cell = &mut buf[(cx, cy)];
            cell.set_char(' ');
            cell.set_bg(btn_bg);
            cell.set_fg(btn_fg);
        }
    }
    let btn_inner = shrink(btn_rect, 2, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            btn_label,
            Style::default()
                .fg(btn_fg)
                .bg(btn_bg)
                .add_modifier(Modifier::BOLD),
        ))),
        btn_inner,
    );
}

/// Returns (label, key) for the primary action button. Color is always WARNING.
fn primary_button(app: &App) -> (&'static str, &'static str) {
    if let Some(ref dl) = app.download {
        if dl.game_id == app.selected_game() {
            return (dl.op_label, "p");
        }
        // Another game is downloading — show what the button would do if enabled
        let game = app.selected_game();
        let status = app.games.get(&game);
        let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
        let has_update = status
            .and_then(|s| s.update_info.as_ref())
            .is_some_and(|i| i.update_available);
        let has_resume = status.is_some_and(|s| s.has_resume);
        if has_resume {
            return ("Resume", "⏎");
        } else if has_update {
            return ("Update", "⏎");
        } else if installed {
            return ("Launch", "⏎");
        } else {
            return ("Get Game", "⏎");
        }
    }
    match app.current_view {
        View::Settings => ("Settings", "s"),
        _ => {
            let game = app.selected_game();
            let status = app.games.get(&game);
            let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
            let has_update = status
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available);
            let has_resume = status.is_some_and(|s| s.has_resume);

            if has_resume {
                ("Resume", "⏎")
            } else if has_update {
                ("Update", "⏎")
            } else if installed {
                ("Launch", "⏎")
            } else {
                ("Get Game", "⏎")
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
        format!("{}:{:02}:{:02}", h, m, sec)
    } else {
        format!("{}:{:02}", m, sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_kb() {
        assert_eq!(format_bytes(512), "0.5 KB");
        assert_eq!(format_bytes(1024), "1.0 KB");
    }

    #[test]
    fn format_bytes_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(52_428_800), "50.0 MB");
    }

    #[test]
    fn format_bytes_gb() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(108_000_000_000), "100.6 GB");
    }

    #[test]
    fn eta_seconds_only() {
        assert_eq!(format_eta_long(0.0), "0:00");
        assert_eq!(format_eta_long(5.0), "0:05");
        assert_eq!(format_eta_long(59.0), "0:59");
    }

    #[test]
    fn eta_minutes() {
        assert_eq!(format_eta_long(60.0), "1:00");
        assert_eq!(format_eta_long(90.0), "1:30");
        assert_eq!(format_eta_long(381.0), "6:21");
    }

    #[test]
    fn eta_hours() {
        assert_eq!(format_eta_long(3600.0), "1:00:00");
        assert_eq!(format_eta_long(6751.0), "1:52:31");
    }

    #[test]
    fn primary_button_not_installed() {
        let config = crate::config::Config::default();
        let app = crate::app::App::new(config);
        let (label, key) = primary_button(&app);
        assert_eq!(label, "Get Game");
        assert_eq!(key, "⏎");
    }

    #[test]
    fn primary_button_installed() {
        let config = crate::config::Config::default();
        let mut app = crate::app::App::new(config);
        let game = app.selected_game();
        app.games.insert(
            game,
            crate::app::GameStatus {
                installed_tag: Some("1.0.0".to_owned()),
                update_info: None,
                has_resume: false,
            },
        );
        let (label, key) = primary_button(&app);
        assert_eq!(label, "Launch");
        assert_eq!(key, "⏎");
    }

    #[test]
    fn primary_button_has_resume() {
        let config = crate::config::Config::default();
        let mut app = crate::app::App::new(config);
        let game = app.selected_game();
        app.games.insert(
            game,
            crate::app::GameStatus {
                installed_tag: None,
                update_info: None,
                has_resume: true,
            },
        );
        let (label, key) = primary_button(&app);
        assert_eq!(label, "Resume");
        assert_eq!(key, "⏎");
    }

    #[test]
    fn primary_button_downloading_same_game() {
        let config = crate::config::Config::default();
        let mut app = crate::app::App::new(config);
        let game = app.selected_game();
        let handle = irmin::DownloadHandle::new();
        app.start_download(game, handle, "Downloading...");
        let (label, key) = primary_button(&app);
        assert_eq!(label, "Downloading...");
        assert_eq!(key, "p");
    }

    #[test]
    fn primary_button_downloading_other_game_installed() {
        let config = crate::config::Config::default();
        let mut app = crate::app::App::new(config);
        // Install the selected game (index 0 = Bh3 by default... actually default is Hk4e)
        let selected = app.selected_game();
        app.games.insert(
            selected,
            crate::app::GameStatus {
                installed_tag: Some("1.0.0".to_owned()),
                update_info: None,
                has_resume: false,
            },
        );
        // Start download on a different game
        let other = crate::game::GameId::Nap;
        let handle = irmin::DownloadHandle::new();
        app.start_download(other, handle, "Downloading...");
        let (label, key) = primary_button(&app);
        assert_eq!(label, "Launch");
        assert_eq!(key, "⏎");
    }
}
