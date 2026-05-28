//! TestBackend-based headless harness for Config TUI integration tests.
//!
//! Wraps `tayf::__test_api` so individual test files do not have to know
//! how `App` is constructed internally. The `App` is intentionally
//! opaque (`AppHandle`) — tests interact only through harness helpers,
//! mirroring how a user interacts through the keyboard.

#![allow(dead_code)] // not all helpers used in every test file

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use tayf::__test_api::{boot_app_with_sample, draw_app, AppHandle};

/// Boot an App with the given sample text on an empty snapshot.
pub fn boot_tui_with_sample(sample: &str) -> (AppHandle, Terminal<TestBackend>) {
    boot_tui_with_sample_sized(sample, 80, 24)
}

/// Boot an App with a caller-specified TestBackend size. Useful for modal
/// overlays whose 80% × 24% centered rect renders too few rows under the
/// default 80×24 to surface multi-line content (e.g. Help modal).
pub fn boot_tui_with_sample_sized(
    sample: &str,
    cols: u16,
    rows: u16,
) -> (AppHandle, Terminal<TestBackend>) {
    let app = boot_app_with_sample(sample);
    let backend = TestBackend::new(cols, rows);
    let terminal = Terminal::new(backend).expect("TestBackend init");
    (app, terminal)
}

/// Draw the current App state into the terminal's buffer and return a clone.
pub fn draw_frame(app: &AppHandle, terminal: &mut Terminal<TestBackend>) -> Buffer {
    draw_app(app, terminal).expect("draw");
    terminal.backend().buffer().clone()
}

/// Find the first occurrence of `needle` text, returning (col, row).
pub fn find_text(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
    let area = buf.area;
    for row in 0..area.height {
        let mut line = String::new();
        for col in 0..area.width {
            line.push_str(buf[(col, row)].symbol());
        }
        if let Some(pos) = line.find(needle) {
            let col = u16::try_from(pos).ok()?;
            return Some((col, row));
        }
    }
    None
}

/// Inspect a buffer cell's style.
pub fn cell_style(buf: &Buffer, col: u16, row: u16) -> ratatui::style::Style {
    buf[(col, row)].style()
}

/// Extract the full text of each row as Vec<String>.
pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    let mut lines = Vec::new();
    let area = buf.area;
    for row in 0..area.height {
        let mut line = String::new();
        for col in 0..area.width {
            line.push_str(buf[(col, row)].symbol());
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}
