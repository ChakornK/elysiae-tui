use std::io;
use std::path::Path;

use image::RgbImage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

// Quadrant block characters indexed by 4-bit pattern.
// Bit layout: [top-left, top-right, bottom-left, bottom-right].
const QUADRANT_CHARS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

/// Pre-rendered quadrant image ready for direct buffer rendering.
#[derive(Clone)]
pub struct QuadrantImage {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<QuadrantCell>,
}

#[derive(Clone)]
pub struct QuadrantCell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

impl QuadrantImage {
    /// Encodes an RGB image into quadrant blocks.
    /// Input image must be (cell_width * 2) x (cell_height * 2) pixels.
    pub fn encode(img: &RgbImage, cell_width: u16, cell_height: u16) -> Self {
        let mut cells = Vec::with_capacity((cell_width as usize) * (cell_height as usize));

        for cy in 0..cell_height as u32 {
            for cx in 0..cell_width as u32 {
                let px = cx * 2;
                let py = cy * 2;
                let colors = [
                    get_pixel(img, px, py),
                    get_pixel(img, px + 1, py),
                    get_pixel(img, px, py + 1),
                    get_pixel(img, px + 1, py + 1),
                ];
                let (ch, fg, bg) = best_quadrant(&colors);
                cells.push(QuadrantCell { ch, fg, bg });
            }
        }

        Self {
            width: cell_width,
            height: cell_height,
            cells,
        }
    }

    /// Darkens all cells by 50% (blend with black). Mutates in place.
    pub fn darken(&mut self) {
        for cell in &mut self.cells {
            if let Color::Rgb(r, g, b) = cell.fg {
                cell.fg = Color::Rgb(r / 2, g / 2, b / 2);
            }
            if let Color::Rgb(r, g, b) = cell.bg {
                cell.bg = Color::Rgb(r / 2, g / 2, b / 2);
            }
        }
    }

    /// Writes the encoded image to a binary cache file.
    pub fn write_cache(&self, path: &Path) -> io::Result<()> {
        let cell_count = self.cells.len();
        let mut buf = Vec::with_capacity(4 + cell_count * 7);
        buf.extend_from_slice(&self.width.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        for cell in &self.cells {
            buf.push(char_to_index(cell.ch));
            let (fr, fg, fb) = rgb_from_color(cell.fg);
            let (br, bg, bb) = rgb_from_color(cell.bg);
            buf.extend_from_slice(&[fr, fg, fb, br, bg, bb]);
        }
        std::fs::write(path, &buf)
    }

    /// Reads a cached quadrant image from disk.
    pub fn read_cache(path: &Path) -> io::Result<Self> {
        let buf = std::fs::read(path)?;
        if buf.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too short"));
        }
        let width = u16::from_le_bytes([buf[0], buf[1]]);
        let height = u16::from_le_bytes([buf[2], buf[3]]);
        let expected = 4 + (width as usize) * (height as usize) * 7;
        if buf.len() != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "size mismatch"));
        }
        let mut cells = Vec::with_capacity((width as usize) * (height as usize));
        let mut i = 4;
        while i + 7 <= buf.len() {
            let ch = QUADRANT_CHARS[(buf[i] & 0x0F) as usize];
            let fg = Color::Rgb(buf[i + 1], buf[i + 2], buf[i + 3]);
            let bg = Color::Rgb(buf[i + 4], buf[i + 5], buf[i + 6]);
            cells.push(QuadrantCell { ch, fg, bg });
            i += 7;
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }
}

impl Widget for &QuadrantImage {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for cy in 0..self.height.min(area.height) {
            for cx in 0..self.width.min(area.width) {
                let idx = (cy as usize) * (self.width as usize) + (cx as usize);
                let Some(cell) = self.cells.get(idx) else {
                    return;
                };
                let buf_cell = &mut buf[(area.x + cx, area.y + cy)];
                buf_cell.set_char(cell.ch);
                buf_cell.set_fg(cell.fg);
                buf_cell.set_bg(cell.bg);
            }
        }
    }
}

/// Picks the best quadrant character + fg/bg for 4 sub-pixel colors.
fn best_quadrant(colors: &[[u8; 3]; 4]) -> (char, Color, Color) {
    // Early exit for uniform cells
    if colors[0] == colors[1] && colors[1] == colors[2] && colors[2] == colors[3] {
        let c = colors[0];
        let color = Color::Rgb(c[0], c[1], c[2]);
        return ('█', color, color);
    }

    // Precompute per-pixel values as u32 for sum arithmetic
    let px: [[u32; 3]; 4] = [
        [
            colors[0][0] as u32,
            colors[0][1] as u32,
            colors[0][2] as u32,
        ],
        [
            colors[1][0] as u32,
            colors[1][1] as u32,
            colors[1][2] as u32,
        ],
        [
            colors[2][0] as u32,
            colors[2][1] as u32,
            colors[2][2] as u32,
        ],
        [
            colors[3][0] as u32,
            colors[3][1] as u32,
            colors[3][2] as u32,
        ],
    ];
    let total = [
        px[0][0] + px[1][0] + px[2][0] + px[3][0],
        px[0][1] + px[1][1] + px[2][1] + px[3][1],
        px[0][2] + px[1][2] + px[2][2] + px[3][2],
    ];
    // Precompute sum of squares for error calculation
    let total_sq: u32 = px
        .iter()
        .map(|p| p[0] * p[0] + p[1] * p[1] + p[2] * p[2])
        .sum();

    let mut best_err = u64::MAX;
    let mut best_pattern = 0u8;

    for pattern in 0u8..16 {
        let fg_count = pattern.count_ones();
        let bg_count = 4 - fg_count;

        // Compute fg_sum by accumulating set bits
        let mut fg_sum = [0u32; 3];
        for i in 0..4u8 {
            if pattern & (1 << i) != 0 {
                fg_sum[0] += px[i as usize][0];
                fg_sum[1] += px[i as usize][1];
                fg_sum[2] += px[i as usize][2];
            }
        }
        let bg_sum = [
            total[0] - fg_sum[0],
            total[1] - fg_sum[1],
            total[2] - fg_sum[2],
        ];

        // Error = total_sq - fg_sum²/fg_count - bg_sum²/bg_count
        // This is the within-group sum of squared deviations
        let fg_err = if fg_count > 0 {
            let sq = fg_sum[0] * fg_sum[0] + fg_sum[1] * fg_sum[1] + fg_sum[2] * fg_sum[2];
            sq as u64 / fg_count as u64
        } else {
            0
        };
        let bg_err = if bg_count > 0 {
            let sq = bg_sum[0] * bg_sum[0] + bg_sum[1] * bg_sum[1] + bg_sum[2] * bg_sum[2];
            sq as u64 / bg_count as u64
        } else {
            0
        };
        let err = total_sq as u64 - fg_err - bg_err;

        if err < best_err {
            best_err = err;
            best_pattern = pattern;
        }
    }

    // Reconstruct fg/bg colors for the winning pattern
    let fg_count = best_pattern.count_ones();
    let bg_count = 4 - fg_count;
    let mut fg_sum = [0u32; 3];
    for i in 0..4u8 {
        if best_pattern & (1 << i) != 0 {
            fg_sum[0] += px[i as usize][0];
            fg_sum[1] += px[i as usize][1];
            fg_sum[2] += px[i as usize][2];
        }
    }

    #[allow(clippy::manual_checked_ops)]
    let fg = if fg_count > 0 {
        Color::Rgb(
            (fg_sum[0] / fg_count) as u8,
            (fg_sum[1] / fg_count) as u8,
            (fg_sum[2] / fg_count) as u8,
        )
    } else {
        Color::Rgb(0, 0, 0)
    };
    #[allow(clippy::manual_checked_ops)]
    let bg = if bg_count > 0 {
        let bg_sum = [
            total[0] - fg_sum[0],
            total[1] - fg_sum[1],
            total[2] - fg_sum[2],
        ];
        Color::Rgb(
            (bg_sum[0] / bg_count) as u8,
            (bg_sum[1] / bg_count) as u8,
            (bg_sum[2] / bg_count) as u8,
        )
    } else {
        Color::Rgb(0, 0, 0)
    };

    (QUADRANT_CHARS[best_pattern as usize], fg, bg)
}

fn char_to_index(ch: char) -> u8 {
    QUADRANT_CHARS.iter().position(|&c| c == ch).unwrap_or(0) as u8
}

fn rgb_from_color(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    }
}

fn get_pixel(img: &RgbImage, x: u32, y: u32) -> [u8; 3] {
    if x < img.width() && y < img.height() {
        img.get_pixel(x, y).0
    } else {
        [0, 0, 0]
    }
}
