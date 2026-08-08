use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Tracks an active ripple fade between two game backgrounds.
pub struct BgTransition {
    pub from_game: GameId,
    started: Instant,
    duration: Duration,
}

impl BgTransition {
    const DEFAULT_DURATION: Duration = Duration::from_millis(400);

    pub fn new(from_game: GameId) -> Self {
        Self {
            from_game,
            started: Instant::now(),
            duration: Self::DEFAULT_DURATION,
        }
    }

    /// Normalized progress clamped to [0.0, 1.0].
    pub fn progress(&self) -> f32 {
        let elapsed = self.started.elapsed().as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).min(1.0)
    }

    pub fn is_done(&self) -> bool {
        self.started.elapsed() >= self.duration
    }
}

/// Renders a ripple-blended frame directly into the buffer.
/// Expands a circular wavefront from center, blending cells at the edge.
pub fn render_ripple_transition(
    from: &QuadrantImage,
    to: &QuadrantImage,
    progress: f32,
    area: Rect,
    buf: &mut Buffer,
) {
    let w = from.width.min(area.width) as f32;
    let h = from.height.min(area.height) as f32;
    let center_x = w / 2.0;
    let center_y = h / 2.0;
    // Cells are ~2x taller than wide; scale height to produce a circular ripple
    const H_SCALE: f32 = 0.5;
    let scaled_cx = center_x * H_SCALE;
    let max_dist = (scaled_cx * scaled_cx + center_y * center_y).sqrt();

    // Overshoot so the fade band fully exits screen before progress=1.0
    let wavefront = progress * 1.3;
    const FADE_WIDTH: f32 = 0.15;

    let cols = from.width.min(area.width);
    let rows = from.height.min(area.height);

    for row in 0..rows {
        for col in 0..cols {
            let idx = (row as usize) * (from.width as usize) + (col as usize);
            let dx = (col as f32 - center_x) * H_SCALE;
            let dy = row as f32 - center_y;
            let norm_dist = (dx * dx + dy * dy).sqrt() / max_dist;

            let buf_cell = &mut buf[(area.x + col, area.y + row)];

            if norm_dist < wavefront - FADE_WIDTH {
                // Inside ripple: new background
                let cell = &to.cells[idx];
                buf_cell.set_char(cell.ch);
                buf_cell.set_fg(cell.fg);
                buf_cell.set_bg(cell.bg);
            } else if norm_dist > wavefront {
                // Outside ripple: old background
                let cell = &from.cells[idx];
                buf_cell.set_char(cell.ch);
                buf_cell.set_fg(cell.fg);
                buf_cell.set_bg(cell.bg);
            } else {
                // Fade band: blend between old and new
                let t = 1.0 - (norm_dist - (wavefront - FADE_WIDTH)) / FADE_WIDTH;
                let fc = &from.cells[idx];
                let tc = &to.cells[idx];
                let ch = if t >= 0.5 { tc.ch } else { fc.ch };
                let fg = lerp_color(fc.fg, tc.fg, t);
                let bg = lerp_color(fc.bg, tc.bg, t);
                buf_cell.set_char(ch);
                buf_cell.set_fg(fg);
                buf_cell.set_bg(bg);
            }
        }
    }
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let r = (ar as f32 + (br as f32 - ar as f32) * t) as u8;
            let g = (ag as f32 + (bg as f32 - ag as f32) * t) as u8;
            let bv = (ab as f32 + (bb as f32 - ab as f32) * t) as u8;
            Color::Rgb(r, g, bv)
        }
        _ => b,
    }
}
