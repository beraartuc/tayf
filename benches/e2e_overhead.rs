//! End-to-end PTY-vs-`cat` overhead measurement harness (spec §7).
//!
//! Drives the release `tayf` binary inside a real PTY and times the
//! streaming phase of `cat <corpus>` against a bare `/bin/sh` running the
//! same command. Reports three overhead columns per corpus shape:
//!
//! - `bypass_ovh%` — pure I/O-loop overhead (`tayf --bypass` vs `cat`)
//! - `pipe_cost%`  — pipeline-internal cost (`tayf` full vs `tayf --bypass`)
//! - `full_ovh%`   — total overhead (`tayf` full vs `cat`; spec §7 target <20%)
//!
//! NOT a criterion bench (`harness = false`): criterion's iteration model
//! fits in-process microbenchmarks, not subprocess + PTY spawning. This is
//! a custom `fn main()` measurement tool, run locally with
//! `cargo bench --bench e2e_overhead`. Numbers are recorded by hand in
//! `benches/BASELINE.md`. Timing covers the streaming phase only (stdin
//! write -> EOF); process spawn and the ~200 ms startup grace are excluded
//! symmetrically from all three sides (see spec §5.5).
//!
//! Tuning env vars: `TAYF_E2E_SAMPLES` (default 10), `TAYF_E2E_WARMUP`
//! (default 3), `TAYF_E2E_BYTES` (default 16 MiB).

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

#[path = "common/math.rs"]
mod math;
use math::{max_sample, median, min_sample, overhead_pct};

/// Fixed PTY geometry for every run (matches the integration-test default).
const PTY_SIZE: PtySize = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

/// Time to let the child install handlers / spawn its shell before we write
/// stdin. Excluded from the measured window (applied identically to both
/// sides). Mirrors `tests/common::spawn_with_input`.
const STARTUP_GRACE: Duration = Duration::from_millis(200);

/// Safety ceiling so a wedged child cannot hang the harness.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(120);

/// One corpus shape: a template repeated to the target size.
struct Shape {
    name: &'static str,
    template: &'static [u8],
}

const SHAPES: &[Shape] = &[
    Shape { name: "low-match-prose", template: include_bytes!("inputs/e2e_prose.txt") },
    Shape { name: "high-match-log", template: include_bytes!("inputs/e2e_log.txt") },
    Shape { name: "ansi-passthrough", template: include_bytes!("inputs/e2e_ansi.txt") },
];

/// Spawn `cmd args` (with `env` overrides) in a fresh PTY, sleep the startup
/// grace, then time the streaming phase: write `stdin`, drain the master to
/// EOF, return the elapsed streaming duration. Spawn + grace are excluded
/// from the returned duration (spec §5.5).
fn timed_run(cmd: &str, args: &[&str], env: &[(&str, &str)], stdin: &str) -> Duration {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PTY_SIZE).expect("openpty");

    let mut builder = CommandBuilder::new(cmd);
    for a in args {
        builder.arg(a);
    }
    for (k, v) in env {
        builder.env(k, v);
    }
    let mut child = pair.slave.spawn_command(builder).expect("spawn");
    drop(pair.slave);

    // Let the child become ready BEFORE the measured window opens.
    std::thread::sleep(STARTUP_GRACE);

    let mut writer = pair.master.take_writer().expect("take writer");
    let mut reader = pair.master.try_clone_reader().expect("clone reader");

    let start = Instant::now();
    writer.write_all(stdin.as_bytes()).expect("write stdin");
    drop(writer);

    let mut buf = [0u8; 65536];
    loop {
        if start.elapsed() > DRAIN_TIMEOUT {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Drain whatever the kernel still has buffered, then stop.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
            break;
        }
    }
    let elapsed = start.elapsed();
    let _ = child.kill();
    let _ = child.wait();
    elapsed
}

/// Repeat `template` until it reaches at least `target` bytes, write it to a
/// fresh temp file, and return the handle (kept alive so the path stays
/// valid). `cat` reads it read-only — no ETXTBSY risk (it is data, not an
/// executable).
fn write_corpus(template: &[u8], target: usize) -> tempfile::NamedTempFile {
    let reps = (target / template.len().max(1)).max(1);
    let mut file = tempfile::NamedTempFile::new().expect("create temp corpus");
    for _ in 0..reps {
        file.write_all(template).expect("write corpus");
    }
    file.flush().expect("flush corpus");
    file
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // reason: byte counts and ms fit f64/u64 for display
fn main() {
    let samples = env_usize("TAYF_E2E_SAMPLES", 10).max(1);
    let warmup = env_usize("TAYF_E2E_WARMUP", 3);
    let target = env_usize("TAYF_E2E_BYTES", 16 * 1024 * 1024);
    let tayf = env!("CARGO_BIN_EXE_tayf");

    println!("tayf v0.8.1 end-to-end overhead (spec §7 target: <20% full vs native cat)");
    println!("samples={samples} warmup={warmup} target_bytes={target}");
    println!("timing = streaming phase only (spawn + {STARTUP_GRACE:?} grace excluded, symmetric)");
    println!();
    println!(
        "{:<18} {:>10} {:>10} {:>10} {:>12} {:>12} {:>8}",
        "shape", "cat_ms", "bypass_ms", "tayf_ms", "bypass_ovh%", "pipe_cost%", "full_ovh%"
    );

    for shape in SHAPES {
        let corpus = write_corpus(shape.template, target);
        // NamedTempFile paths are alphanumeric under the temp dir (no spaces
        // or shell metacharacters), so the path needs no shell quoting here.
        let stdin = format!("cat {}\nexit\n", corpus.path().display());

        let cat_args: &[&str] = &[];
        let cat_env: &[(&str, &str)] = &[];
        let bypass_args: &[&str] = &["--bypass", "--shell", "/bin/sh"];
        let bypass_env: &[(&str, &str)] = &[("TAYF_DISABLE_BG_DETECT", "1")];
        let tayf_args: &[&str] = &["--shell", "/bin/sh"];
        let tayf_env: &[(&str, &str)] = &[("TAYF_DISABLE_BG_DETECT", "1")];

        for _ in 0..warmup {
            timed_run("/bin/sh", cat_args, cat_env, &stdin);
            timed_run(tayf, bypass_args, bypass_env, &stdin);
            timed_run(tayf, tayf_args, tayf_env, &stdin);
        }

        let mut cat_ms = Vec::with_capacity(samples);
        let mut bypass_ms = Vec::with_capacity(samples);
        let mut tayf_ms = Vec::with_capacity(samples);
        for _ in 0..samples {
            cat_ms.push(timed_run("/bin/sh", cat_args, cat_env, &stdin).as_secs_f64() * 1000.0);
            bypass_ms.push(timed_run(tayf, bypass_args, bypass_env, &stdin).as_secs_f64() * 1000.0);
            tayf_ms.push(timed_run(tayf, tayf_args, tayf_env, &stdin).as_secs_f64() * 1000.0);
        }

        let cat_med = median(&cat_ms);
        let bypass_med = median(&bypass_ms);
        let tayf_med = median(&tayf_ms);
        let full_over = overhead_pct(tayf_med, cat_med);
        let bypass_over = overhead_pct(bypass_med, cat_med);
        let pipeline_cost = overhead_pct(tayf_med, bypass_med);
        let result = if full_over < 20.0 { "PASS" } else { "FAIL" };

        println!(
            "{:<18} {:>10.2} {:>10.2} {:>10.2} {:>11.2}% {:>11.2}% {:>8.2}% [{}]",
            shape.name,
            cat_med,
            bypass_med,
            tayf_med,
            bypass_over,
            pipeline_cost,
            full_over,
            result
        );
        eprintln!(
            "  {} detail: cat [min {:.2}/med {:.2}/max {:.2}] bypass [min {:.2}/med {:.2}/max {:.2}] tayf [min {:.2}/med {:.2}/max {:.2}]",
            shape.name,
            min_sample(&cat_ms), cat_med, max_sample(&cat_ms),
            min_sample(&bypass_ms), bypass_med, max_sample(&bypass_ms),
            min_sample(&tayf_ms), tayf_med, max_sample(&tayf_ms),
        );
    }
}
