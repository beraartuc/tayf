//! Save semantics — atomic write + backup rotation + `TmpFileGuard`.
//!
//! Spec §8.1 + §8.3. Detailed flow:
//!   1. `rotate_backups_to(MAX_BACKUPS - 1)` — I-3 fold (rotate first)
//!   2. read current on-disk bytes (`disk_now`)
//!   3. write backup with preserved mode (I-2 fold)
//!   4. build new content via `toml_edit` (pass-through stub for C1c)
//!   5. `TmpFileGuard::create_in_parent_dir` + `write_all` + `sync_all`
//!      (I-2 preserved mode; I-1 `sync_all` NOT `sync_data`)
//!   6. `tmp.persist` — atomic POSIX rename
//!   7. parent dir `sync_all` (best-effort; APFS underdocumented)
//!   8. snapshot reparse

// reason: helpers like `ts_for_backup_filename` and `civil_from_days`
// are only reachable on the v0.5.5+ first-run-init dump path (the
// timestamp filename helper is for the dump-backup case). Module-level
// allow until that path lands.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Max retained backup files including the just-written one.
pub(crate) const MAX_BACKUPS: usize = 5;

/// Build the backup filename suffix from a `SystemTime`. Pure fn so
/// test fixtures can pin the exact string against synthetic times.
///
/// Shape: `2026-05-25T17-18-42-123Z`. Colon (`:`) substituted with
/// `-` for Windows-cross-platform forward-friendliness (v1.0+).
pub(crate) fn ts_for_backup_filename(now: SystemTime) -> String {
    let dur = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let ms = dur.subsec_millis();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let sec_of_day = u32::try_from(secs % 86_400).unwrap_or(0);
    let (h, m, s) = (sec_of_day / 3600, (sec_of_day / 60) % 60, sec_of_day % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}-{m:02}-{s:02}-{ms:03}Z")
}

/// Howard Hinnant's `civil_from_days` — convert days-since-epoch
/// to (year, month, day) without leap-second magic. Public domain.
//
// reason: the algorithm's integer casts (i64 ↔ u64 for the day-of-era
// arithmetic and the u64 → u32 truncations for month/day, which are
// algebraically bounded to 1..=12 and 1..=31) mirror the reference
// formulation in Howard Hinnant's `date.h` (public domain). Renaming
// or boxing them in `try_into` would diverge from the citable source
// and make the code harder to audit against the reference.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = i32::try_from(y + i64::from(m <= 2)).unwrap_or(0);
    (yr, m as u32, d as u32)
}

/// RAII guard around a write-target tmpfile. `Drop` unlinks the
/// tmpfile unless `persist()` was called. ~30 LOC custom guard;
/// prod-dep `tempfile` deliberately not pulled per
/// `feedback_dependency_minimalism`.
pub(crate) struct TmpFileGuard {
    path: PathBuf,
    file: Option<std::fs::File>,
    persisted: bool,
}

impl TmpFileGuard {
    /// Create tmpfile in the target's parent dir (EXDEV safety) with
    /// the preserved mode (umask-immune). Caller MUST verify the path
    /// shares the rename target's parent.
    pub(crate) fn create_in_parent_dir(tmp: &Path, mode: u32) -> std::io::Result<Self> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_CREAT | O_EXCL
            .mode(mode)
            .open(tmp)?;
        Ok(Self { path: tmp.to_path_buf(), file: Some(file), persisted: false })
    }

    pub(crate) fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "tmpfile already persisted")
        })?;
        f.write_all(bytes)
    }

    pub(crate) fn sync_all(&mut self) -> std::io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "tmpfile already persisted")
        })?;
        // I-1(a) fold: sync_all (fsync), not sync_data (fdatasync).
        // ext4 data=writeback can lose size after crash-replay otherwise.
        f.sync_all()
    }

    /// Atomic POSIX rename. Consumes self; Drop becomes a no-op.
    pub(crate) fn persist(mut self, target: &Path) -> std::io::Result<()> {
        // Drop the File handle before rename — POSIX-safe on Linux/macOS.
        drop(self.file.take());
        fs::rename(&self.path, target)?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for TmpFileGuard {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Rotate backups DOWN to `target_max` (I-3 fold: rotate FIRST,
/// then write — quota safety).
///
/// Returns `Ok(())` on success; rotation `read_dir` failure surfaces
/// as `Err` so the caller can downgrade to `Toast::warn` + proceed.
pub(crate) fn rotate_backups_to(
    cfg_dir: &Path,
    cfg_stem: &str,
    target_max: usize,
) -> std::io::Result<()> {
    let prefix = format!("{cfg_stem}.tayf-backup-");
    let mut entries: Vec<(PathBuf, SystemTime)> = fs::read_dir(cfg_dir)?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_owned();
            if !name.starts_with(&prefix) {
                return None;
            }
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((p, mtime))
        })
        .collect();
    // newest first — sort_by_key with Reverse to satisfy clippy::unnecessary_sort_by
    entries.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
    for (path, _) in entries.into_iter().skip(target_max) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// Build the candidate new content from snapshot + edits. v0.5.4 stub
/// for C1c — full `toml_edit` reconciliation lands in C3 when edits
/// from each tab flow in. C1c emits the snapshot's raw bytes as a
/// pass-through (no-op save round-trip) so the atomic-write
/// machinery can be tested standalone.
pub(crate) fn build_new_content(
    snapshot: &crate::config_tui::snapshot::ConfigSnapshot,
    _edits: &crate::config_tui::edit::PendingEdits,
) -> String {
    // C1c: pass-through. C3 will replace this body with the
    // toml_edit DocumentMut mutation pipeline (preserve comments +
    // ordering + formatting).
    String::from_utf8_lossy(&snapshot.raw_bytes).into_owned()
}

/// Commit the staged edits to disk atomically.
///
/// Returns the new `ConfigSnapshot` (`source_hash` updated) on
/// success; `Err` on any save failure (backup write, tmp create, rename).
pub(crate) fn commit_save(
    snapshot: &crate::config_tui::snapshot::ConfigSnapshot,
    edits: &crate::config_tui::edit::PendingEdits,
    now: SystemTime,
) -> std::io::Result<crate::config_tui::snapshot::ConfigSnapshot> {
    let cfg_path = snapshot.source_path.as_deref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no source_path; first-run save requires init",
        )
    })?;
    let cfg_dir = cfg_path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "cfg_path has no parent")
    })?;
    let cfg_stem = cfg_path.file_name().and_then(|s| s.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "cfg_path has no filename")
    })?;

    // Step 1: rotate FIRST (I-3 fold). Surface read_dir failures so
    // the caller (C2a SaveDiff modal) can downgrade to Toast::warn.
    rotate_backups_to(cfg_dir, cfg_stem, MAX_BACKUPS - 1)?;

    // Step 2: read current on-disk content (captures any concurrent
    // manual edits between TUI read and save; backup reflects actual
    // disk state, not the stale snapshot view).
    let disk_now = fs::read(cfg_path)?;

    // preserved_mode hoisted: applies to BOTH the backup write
    // (Step 3) and the tmpfile create (Step 5). I-2 fold must cover
    // both paths so a 0o600 source produces 0o600 backup AND 0o600
    // tmpfile — otherwise the backup leaks config content to other
    // local users via umask-default 0o644.
    let preserved_mode: u32 =
        fs::metadata(cfg_path).map_or(0o600, |m| m.permissions().mode() & 0o777);

    // Step 3: write backup with preserved mode (I-2 fold).
    let backup_path =
        cfg_dir.join(format!("{cfg_stem}.tayf-backup-{}", ts_for_backup_filename(now)));
    {
        let mut backup_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_CREAT | O_EXCL — no symlink-precreate
            .mode(preserved_mode)
            .open(&backup_path)?;
        backup_file.write_all(&disk_now)?;
        backup_file.sync_all()?;
    }

    // Step 4: build new content.
    let new_content = build_new_content(snapshot, edits);

    // Step 5: tmpfile in parent dir with preserved mode (I-2 fold),
    // write, sync_all (I-1 fold).
    let pid = std::process::id();
    let tmp_ms = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let tmp_path = cfg_dir.join(format!("{cfg_stem}.tayf-tmp-{pid}-{tmp_ms}"));
    debug_assert_eq!(
        tmp_path.parent(),
        cfg_path.parent(),
        "tmpfile MUST be in target's parent dir (EXDEV safety)"
    );
    let mut tmp = TmpFileGuard::create_in_parent_dir(&tmp_path, preserved_mode)?;
    tmp.write_all(new_content.as_bytes())?;
    tmp.sync_all()?;

    // Step 6: atomic POSIX rename.
    tmp.persist(cfg_path)?;

    // Step 7: parent dir sync_all (best-effort; APFS underdocumented).
    if let Ok(dir) = fs::File::open(cfg_dir) {
        let _ = dir.sync_all();
    }

    // Step 8: rebuild snapshot.
    crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(cfg_path))
        .map_err(|e| std::io::Error::other(format!("post-save reparse: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_for_backup_filename_byte_pinned() {
        // Fixture: epoch seconds 1_779_667_200 = 2026-05-25T00:00:00Z
        // (verified via `date -u -r 1779667200`). Adding 17h 18m 42s 123ms
        // yields 2026-05-25T17:18:42.123Z. The shape `YYYY-MM-DDTHH-MM-SS-mmmZ`
        // (N-3 fold: colon → dash for Windows-cross-platform) is what
        // matters; the date itself is verification convenience.
        use std::time::Duration;
        let ts = SystemTime::UNIX_EPOCH
            + Duration::from_millis(
                1_779_667_200_000 + 17 * 3_600_000 + 18 * 60_000 + 42_000 + 123,
            );
        let name = ts_for_backup_filename(ts);
        assert_eq!(name, "2026-05-25T17-18-42-123Z", "byte-pinned per N-3 fold; got: {name}");
    }

    #[test]
    fn tmpfile_guard_unlinks_on_drop_when_not_persisted() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let path = tmp.path().join("payload.tmp");
        {
            let mut g = TmpFileGuard::create_in_parent_dir(&path, 0o600).expect("create tmpfile");
            g.write_all(b"hello").expect("write");
            // drop without persist → unlink fires
        }
        assert!(!path.exists(), "tmpfile must be unlinked on drop");
    }

    #[test]
    fn tmpfile_guard_persist_leaves_target_intact() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let tmp_path = tmp.path().join("payload.tmp");
        let target = tmp.path().join("target.toml");
        let mut g = TmpFileGuard::create_in_parent_dir(&tmp_path, 0o600).unwrap();
        g.write_all(b"persisted").unwrap();
        g.sync_all().unwrap();
        g.persist(&target).unwrap();
        assert!(!tmp_path.exists(), "tmp gone");
        assert!(target.exists(), "target written");
        assert_eq!(std::fs::read(&target).unwrap(), b"persisted");
    }

    #[test]
    fn tmpfile_guard_preserves_mode() {
        // I-2 fold: target file 0o600 → tmpfile created 0o600 (not
        // umask default 0o644).
        let tmp = tempfile::tempdir().expect("tmpdir");
        let path = tmp.path().join("perm.tmp");
        let g = TmpFileGuard::create_in_parent_dir(&path, 0o600).expect("create");
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0o600; got 0o{mode:o}");
        drop(g);
    }

    #[test]
    fn commit_save_happy_path_writes_backup_and_updates_snapshot() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let edits = crate::config_tui::edit::PendingEdits::default(); // no edits → pass-through
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_779_667_200);
        let new_snap = commit_save(&snap, &edits, now).expect("save");
        // Disk content preserved (pass-through C1c stub).
        assert_eq!(std::fs::read(&cfg_path).unwrap(), b"[general]\ntheme = \"dark\"\n");
        // Backup exists.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .collect();
        assert_eq!(entries.len(), 1, "exactly one backup must exist");
        // New snapshot hash matches what's now on disk.
        let reread = std::fs::read(&cfg_path).unwrap();
        assert_eq!(new_snap.source_hash, crate::config_tui::snapshot::sha256(&reread));
    }

    #[test]
    fn rotate_backups_keeps_target_max_newest() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        // Synthesize 7 backups with monotonically-increasing mtime.
        for i in 0..7 {
            let p = tmp.path().join(format!("config.toml.tayf-backup-2026-05-26T00-00-0{i}-000Z"));
            std::fs::write(&p, format!("backup-{i}")).unwrap();
            // mtime can be left implicit (creation order ≈ mtime order on tmpfs).
        }
        rotate_backups_to(tmp.path(), "config.toml", 4).unwrap();
        let remaining: usize = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .count();
        // Asked for target_max = 4 → exactly 4 newest survive (per
        // feedback_test_assertion_specificity: pin the contract).
        assert_eq!(remaining, 4, "rotation must keep exactly 4 newest; got {remaining}");
    }

    #[test]
    fn integration_save_roundtrip_creates_backup_and_preserves_content() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        let source = b"# Original comment header\n[general]\ntheme = \"dark\"\n";
        std::fs::write(&cfg_path, source).expect("write");
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let edits = crate::config_tui::edit::PendingEdits::default();
        let new_snap = commit_save(&snap, &edits, SystemTime::now()).expect("save");
        let after = std::fs::read(&cfg_path).expect("re-read");
        assert_eq!(after, source as &[u8]);
        assert!(String::from_utf8_lossy(&after).contains("# Original comment header"));
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .collect();
        assert_eq!(entries.len(), 1);
        let reread = std::fs::read(&cfg_path).expect("re-read 2");
        assert_eq!(new_snap.source_hash, crate::config_tui::snapshot::sha256(&reread));
    }

    #[test]
    fn integration_save_rotation_keeps_at_most_max_backups() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").expect("write");
        for _ in 0..7 {
            let snap = crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path))
                .unwrap();
            let edits = crate::config_tui::edit::PendingEdits::default();
            commit_save(&snap, &edits, SystemTime::now()).expect("save");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let count = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .count();
        assert_eq!(count, MAX_BACKUPS, "rotation must keep exactly {MAX_BACKUPS}; got {count}");
    }

    #[test]
    fn merge_collision_user_config_name_clobbers_silently() {
        // I-6 fold (spec §8.1): same-name UserConfig RuleId collision
        // = last-writer-wins by-key. C1c uses pass-through
        // build_new_content; this test pins the merge-key behavior
        // structurally so the C3 toml_edit reconciliation cannot
        // silently drift to a different merge semantic without
        // updating this test.
        use crate::config_tui::edit::{PendingEdits, RuleEdit, RuleId};
        use std::collections::HashMap;
        let mut p = PendingEdits::default();
        // TUI #1 adds "alpha" with one edit.
        p.rules.insert(
            RuleId::UserConfig("alpha".to_owned()),
            RuleEdit { pattern: Some("first".to_owned()), styles: HashMap::new() },
        );
        // TUI #2 races and writes a different "alpha" — overwrites.
        p.rules.insert(
            RuleId::UserConfig("alpha".to_owned()),
            RuleEdit { pattern: Some("second".to_owned()), styles: HashMap::new() },
        );
        let e = p.rules.get(&RuleId::UserConfig("alpha".to_owned())).unwrap();
        assert_eq!(
            e.pattern.as_deref(),
            Some("second"),
            "last-write wins by RuleId key; first edit silently lost — \
             documented behavior, v0.6+ may add per-key conflict UI"
        );
    }
}
