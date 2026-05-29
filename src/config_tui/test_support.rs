//! Snapshot helper for render-layer regression tests.
//!
//! Helper internals use `panic!` and `expect()` by design: snapshot
//! mismatch is an immediate test crash. The lint exception below covers
//! these intentional uses.

#![allow(clippy::expect_used)]
// reason: snapshot helper internals; failures are immediate test crashes by design.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui::Terminal;

use crate::config_tui::app::App;

/// Render an `App` to a fixed-size [`ratatui::backend::TestBackend`], stringify the buffer in
/// plain text, compare against the golden file at
/// `src/config_tui/snapshots/<snap_name>.snap`.
///
/// `UPDATE_SNAPSHOTS=1` env var regenerates the golden (write `rendered`).
/// CI guard: refuses to regenerate when `CI=true` env is set.
pub(crate) fn assert_render_snapshot(
    width: u16,
    height: u16,
    app: &App,
    draw: impl Fn(&mut Frame, Rect, &App),
    snap_name: &str,
) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("TestBackend init");
    terminal.draw(|f| draw(f, Rect::new(0, 0, width, height), app)).expect("draw");
    let rendered = stringify_buffer(terminal.backend().buffer());

    let abs_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/config_tui/snapshots")
        .join(format!("{snap_name}.snap"));

    let update_requested = std::env::var("UPDATE_SNAPSHOTS").is_ok();
    let in_ci = std::env::var("CI").is_ok_and(|v| v == "true" || v == "1");

    assert!(
        !(update_requested && in_ci),
        "UPDATE_SNAPSHOTS=1 refused under CI=true — golden minting only allowed locally"
    );
    if update_requested {
        std::fs::create_dir_all(abs_path.parent().expect("snapshots dir parent"))
            .expect("create snapshots dir");
        std::fs::write(&abs_path, &rendered).expect("write snapshot");
        eprintln!("UPDATED snapshot: {}", abs_path.display());
        return;
    }

    let expected = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| {
        panic!(
            "snapshot not found: {} — rerun with UPDATE_SNAPSHOTS=1 to create",
            abs_path.display(),
        )
    });

    if rendered != expected {
        let diff = crate::config_tui::widgets::save_diff::build_diff(
            expected.as_bytes(),
            rendered.as_bytes(),
        );
        panic!(
            "render snapshot mismatch for {snap_name}:\n{diff}\n\
            re-run with UPDATE_SNAPSHOTS=1 to accept the new output"
        );
    }
}

fn stringify_buffer(buf: &Buffer) -> String {
    // ratatui 0.30 API: `buf.area` field (not method), `buf[(x, y)]` Index (not get()).
    let area = buf.area;
    let mut out = String::with_capacity(
        usize::from(area.width).saturating_mul(usize::from(area.height).saturating_add(1)),
    );
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
