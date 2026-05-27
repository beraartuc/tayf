//! v0.5.3 integration tests for the built-in profile library.
//!
//! Each test spawns `tayf --profile <name>` under a real PTY against
//! an empty `XDG_CONFIG_HOME` (no disk profile shadows the embedded
//! library), feeds a domain-typical input line, and asserts SGR
//! injection on the expected substrings.

#![cfg(unix)]
#![allow(clippy::expect_used)] // reason: tests, not library code

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize};

#[allow(dead_code)] // reason: helper kept for symmetry with sibling files.
fn tayf_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tayf"))
}

fn run_with_profile(xdg: &Path, input: &str, profile: &str) -> Vec<u8> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tayf"));
    cmd.env_remove("HOME");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env("XDG_CONFIG_HOME", xdg);
    cmd.env("TAYF_DISABLE_BG_DETECT", "1");
    cmd.arg("--shell");
    cmd.arg("/bin/sh");
    cmd.arg("--no-hot-reload");
    cmd.arg("--profile");
    cmd.arg(profile);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn tayf");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut writer = pair.master.take_writer().expect("take writer");

    thread::sleep(Duration::from_millis(200));
    let line = format!("printf %s '{input}'\necho\nexit\n");
    writer.write_all(line.as_bytes()).expect("write");
    drop(writer);

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _reader_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match reader.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        let _ = tx.send(buf);
    });

    let _ = child.wait();
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

fn has_some_sgr_around(bytes: &[u8], needle: &str) -> bool {
    let s = String::from_utf8_lossy(bytes);
    s.contains(needle) && s.contains("\u{1b}[")
}

// ---------------------------------------------------------------------------
// aws profile — uses collision-free shapes for envelope verification, plus
// interior-shape verification for canonical EC2 ARN.
// ---------------------------------------------------------------------------

#[test]
fn aws_profile_renders_instance_id_region_on_canonical_ec2_input() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    // Canonical EC2 line — interior region (us-east-2) + instance_id
    // will highlight; aws.arn envelope will NOT (interior overlap).
    let input = "Launching i-0abcd1234567890ef in us-east-2 \
         (arn:aws:ec2:us-east-2:123456789012:instance/i-0abcd1234567890ef)";
    let bytes = run_with_profile(xdg.path(), input, "aws");
    assert!(
        has_some_sgr_around(&bytes, "i-0abcd1234567890ef"),
        "expected styling around instance_id; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        has_some_sgr_around(&bytes, "us-east-2"),
        "expected styling around region; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn aws_profile_renders_arn_envelope_on_collision_free_input() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    // IAM role ARN — no interior region/instance_id/ipv6/ipv4 substring,
    // so aws.arn fires on the envelope.
    let input = "Found arn:aws:iam:::role/MyRole ok";
    let bytes = run_with_profile(xdg.path(), input, "aws");
    assert!(
        has_some_sgr_around(&bytes, "arn:aws:iam:::role/MyRole"),
        "expected styling around collision-free ARN; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

// ---------------------------------------------------------------------------
// k8s profile
// ---------------------------------------------------------------------------

#[test]
fn k8s_profile_renders_pod_name_on_kubectl_output() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let input = "Pod coredns-558bd4d5db-vwz2j Running 5m";
    let bytes = run_with_profile(xdg.path(), input, "k8s");
    assert!(
        has_some_sgr_around(&bytes, "coredns-558bd4d5db-vwz2j"),
        "expected styling around pod_name; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

// ---------------------------------------------------------------------------
// docker profile
// ---------------------------------------------------------------------------

#[test]
fn docker_profile_renders_container_id_and_partial_image_tag() {
    // v0.5.4 E1 retighten: prior test asserted has_some_sgr_around on
    // `nginx:latest` which was satisfied by either correct image_tag
    // styling OR the fqdn-wins limitation pinned by
    // `docker_image_tag_registry_host_yields_to_fqdn_v0_5_3_limitation`.
    // Rename clarifies "partial" (per-segment styling on bare image
    // tag without registry-host prefix) and tightens the assertion
    // to require magenta SGR around the `:` separator specifically,
    // distinguishing the image_tag rule from a generic-token rule.
    let xdg = tempfile::tempdir().expect("tmpdir");
    let input = "Container abc123def456 image=nginx:latest started";
    let bytes = run_with_profile(xdg.path(), input, "docker");
    assert!(
        has_some_sgr_around(&bytes, "abc123def456"),
        "expected styling around container_id; got: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // Tightened: require magenta (FG 35) immediately before
    // `nginx:latest`, not just any SGR anywhere in the output.
    // We search for the exact byte sequence `\x1b[35mnginx:latest`
    // to distinguish the image_tag rule (magenta) from a generic-token
    // rule.  `body.find` could match the un-styled echo of the command,
    // so we scan `bytes` directly for the 4-byte CSI+35m prefix.
    let magenta_prefix = b"\x1b[35m";
    let needle = b"nginx:latest";
    let found = bytes
        .windows(magenta_prefix.len() + needle.len())
        .any(|w| w.starts_with(magenta_prefix) && w.ends_with(needle));
    assert!(
        found,
        "expected magenta (FG 35) SGR immediately before nginx:latest; full: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

// ---------------------------------------------------------------------------
// gcp profile (filter-only — proves whitelist drops `permission`)
// ---------------------------------------------------------------------------

#[test]
fn gcp_profile_filter_only_drops_permission_keeps_whitelist() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    // Input mixes a permission shape (rwxr-xr-x — whitelisted out) with
    // whitelist members (timestamp, log_level).
    let input = "drwxr-xr-x bucket 2026-05-26T10:00:00Z INFO status=200";
    let bytes = run_with_profile(xdg.path(), input, "gcp");
    let s = String::from_utf8_lossy(&bytes);
    // Whitelist members must appear in output. The timestamp built-in
    // uses capture-group styling (date / sep / time / ms / tz — five
    // SGR runs per match), so the full ISO literal is NOT contiguous in
    // the styled bytes. The date capture group (`\d{4}-\d{2}-\d{2}`)
    // IS contiguous within its own yellow SGR; checking that proves the
    // timestamp rule fired without depending on inter-group bytes.
    assert!(s.contains("2026-05-26"), "timestamp date capture-group must survive");
    assert!(s.contains("INFO"), "log_level text must survive");
    assert!(s.contains("200"), "plain status=200 text must pass through");
    // permission rule (`rwxr-xr-x`) must appear plainly (not styled).
    assert!(
        s.contains("drwxr-xr-x"),
        "perm shape must appear plain (whitelist filter dropped permission rule)"
    );
}

// ---------------------------------------------------------------------------
// network profile (filter-only — proves whitelist drops `uuid`)
// ---------------------------------------------------------------------------

#[test]
fn network_profile_filter_only_drops_uuid_keeps_ips() {
    let xdg = tempfile::tempdir().expect("tmpdir");
    let input = "192.168.1.1:443 -> 10.0.0.2:8080 \
         (uuid 123e4567-e89b-12d3-a456-426614174000)";
    let bytes = run_with_profile(xdg.path(), input, "network");
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("192.168.1.1"), "ipv4 #1 must surface");
    assert!(s.contains("10.0.0.2"), "ipv4 #2 must surface");
    assert!(
        s.contains("123e4567-e89b-12d3-a456-426614174000"),
        "uuid must appear literally (whitelist filter dropped uuid rule)"
    );
    assert!(s.contains("\u{1b}["), "expected at least one SGR; got: {s:?}");
}
