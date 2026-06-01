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

/// Build the candidate new content from snapshot + edits. v0.5.5: thin
/// facade into [`crate::config_tui::reconcile::apply_edits`] — the
/// [`crate::config_tui::edit::PendingEdits`] → [`toml_edit::DocumentMut`] walk lives in
/// `reconcile.rs`. This module stays focused on atomic-write + backup-rotation semantics.
pub(crate) fn build_new_content(
    snapshot: &crate::config_tui::snapshot::ConfigSnapshot,
    edits: &crate::config_tui::edit::PendingEdits,
) -> Result<String, crate::config_tui::reconcile::ReconcileError> {
    crate::config_tui::reconcile::apply_edits(&snapshot.doc, edits)
}

/// Commit a pre-built TOML body to disk atomically — the 8-step
/// ceremony shared between [`commit_save`] (Clean Confirm path) and the
/// G8 merge-conflict apply path (`apply_conflict_selections` in
/// `events.rs`). Centralizing the write ensures every config write
/// preserves the source mode, rotates backups, and goes through
/// `sync_all`. Memory `feedback_parallel_call_site_invariant_audit` —
/// invariants must apply to ALL effectful write paths, not a subset.
///
/// Returns the new `ConfigSnapshot` (`source_hash` updated) on success;
/// `Err` on any save failure (backup write, tmp create, rename).
pub(crate) fn commit_bytes(
    snapshot: &crate::config_tui::snapshot::ConfigSnapshot,
    body: &str,
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

    // Step 4: tmpfile in parent dir with preserved mode (I-2 fold),
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
    tmp.write_all(body.as_bytes())?;
    tmp.sync_all()?;

    // Step 5: atomic POSIX rename.
    tmp.persist(cfg_path)?;

    // Step 6: parent dir sync_all (best-effort; APFS underdocumented).
    if let Ok(dir) = fs::File::open(cfg_dir) {
        let _ = dir.sync_all();
    }

    // Step 7: rebuild snapshot.
    crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(cfg_path))
        .map_err(|e| std::io::Error::other(format!("post-save reparse: {e}")))
}

/// Commit the staged edits to disk atomically. Thin facade over
/// [`commit_bytes`] — builds the new content from `snapshot.doc + edits`
/// then delegates the write.
pub(crate) fn commit_save(
    snapshot: &crate::config_tui::snapshot::ConfigSnapshot,
    edits: &crate::config_tui::edit::PendingEdits,
    now: SystemTime,
) -> std::io::Result<crate::config_tui::snapshot::ConfigSnapshot> {
    let body = build_new_content(snapshot, edits)
        .map_err(|e| std::io::Error::other(format!("reconcile failed: {e}")))?;
    commit_bytes(snapshot, &body, now)
}

/// Atomically write `content` to `target` using a tmpfile + rename
/// strategy. Mirrors [`TmpFileGuard::persist`] semantics applied to a
/// string body. Used by the `Shift+D` init-from-dump flow (v0.6.1 §3.3).
///
/// Creates `target.parent()` if it does not exist. The tmpfile is
/// created in the target's parent directory (EXDEV safety) with mode
/// `0o600` (init-from-dump is the only path that creates a fresh
/// config file; preserved-mode does not apply because no prior file
/// exists to read the mode from).
/// Verify `dest` is safe to write through, given `tayf_root` as the
/// canonical config-tree root (e.g. `~/.config/tayf/`). Two-layer gate:
///
/// 1. **lstat:** the destination itself must not be a symlink.
///    `rename(2)` would dereference one and overwrite the target — a
///    user could craft `~/.config/tayf/profiles/aws.toml -> /etc/passwd`
///    and have tayf silently clobber it. Refuse outright.
/// 2. **Canonical parent:** the destination's parent directory, after
///    `canonicalize`, must lie under `tayf_root` (also canonicalized).
///    Protects against a symlinked `profiles/` directory pointing
///    outside the tayf tree.
///
/// Returns a human-readable rejection reason in the `Err` case — the
/// caller surfaces it as a TUI toast. CLAUDE.md §3 mandate.
///
/// # Errors
/// `Err(reason)` when either gate trips, the parent cannot be created,
/// or canonicalization fails. Treats `dest` not existing as fine — the
/// override-copy path writes a *new* file.
pub(crate) fn check_safe_write_destination(dest: &Path, tayf_root: &Path) -> Result<(), String> {
    // 1. lstat — does NOT follow symlinks
    match dest.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("dest is a symlink: {}", dest.display()));
        }
        Ok(_) | Err(_) if !dest.exists() => {
            // dest absent — fall through; the write_atomic_to path will
            // create it (and `rename(2)` does not follow nonexistent
            // targets).
        }
        Ok(_) => {}
        Err(e) => return Err(format!("dest stat failed: {e}")),
    }

    // 2. Canonical parent inside canonical tayf_root
    let Some(parent) = dest.parent() else {
        return Err("dest has no parent directory".to_owned());
    };
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent failed: {e}"))?;
    let canonical_parent =
        parent.canonicalize().map_err(|e| format!("canonicalize parent failed: {e}"))?;
    let canonical_root =
        tayf_root.canonicalize().map_err(|e| format!("canonicalize tayf_root failed: {e}"))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!(
            "parent {} resolves outside tayf root {}",
            canonical_parent.display(),
            canonical_root.display(),
        ));
    }
    Ok(())
}

/// Canonical `~/.config/tayf/` resolved from `$XDG_CONFIG_HOME` (preferred)
/// or `$HOME/.config`. Returns `None` when neither environment variable
/// is set. The returned path is NOT canonicalized (it may not exist
/// yet); call `canonicalize` if you need the resolved filesystem path.
///
/// Centralized here so the profile / theme override-copy path and any
/// future writer share a single env-resolution policy.
pub(crate) fn tayf_config_root() -> Option<PathBuf> {
    let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".config")
    };
    Some(base.join("tayf"))
}

pub(crate) fn write_atomic_to(target: &Path, content: &str) -> std::io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "target path has no parent directory")
    })?;
    fs::create_dir_all(parent)?;
    let pid = std::process::id();
    let stamp =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let stem = target.file_name().and_then(|s| s.to_str()).unwrap_or("config.toml");
    let tmp_path = parent.join(format!("{stem}.tayf-init-{pid}-{stamp}"));
    let mut tmp = TmpFileGuard::create_in_parent_dir(&tmp_path, 0o600)?;
    tmp.write_all(content.as_bytes())?;
    tmp.sync_all()?;
    tmp.persist(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // G6 — check_safe_write_destination symlink + canonical-parent gate.
    // CLAUDE.md §3 mandate: "reject symlink traversal outside ~/.config/tayf/".
    // -----------------------------------------------------------------------

    #[test]
    fn check_safe_write_destination_accepts_path_inside_canonical_root() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let tayf_root = tmp.path().join("tayf");
        std::fs::create_dir_all(tayf_root.join("profiles")).expect("mkdir");
        let dest = tayf_root.join("profiles").join("aws.toml");

        check_safe_write_destination(&dest, &tayf_root)
            .expect("path under canonical tayf root is safe");
    }

    #[test]
    fn check_safe_write_destination_rejects_when_dest_is_symlink() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let tayf_root = tmp.path().join("tayf");
        let profiles_dir = tayf_root.join("profiles");
        std::fs::create_dir_all(&profiles_dir).expect("mkdir");
        let outside = tmp.path().join("outside.toml");
        std::fs::write(&outside, "stolen-target").expect("write outside");
        let dest = profiles_dir.join("aws.toml");
        std::os::unix::fs::symlink(&outside, &dest).expect("symlink");

        let err = check_safe_write_destination(&dest, &tayf_root)
            .expect_err("symlink dest must be rejected");
        assert!(err.contains("symlink"), "rejection reason must mention 'symlink'; got: {err}");
        // Outside file must NOT have been touched by the check itself.
        assert_eq!(std::fs::read_to_string(&outside).expect("read outside"), "stolen-target");
    }

    #[test]
    fn check_safe_write_destination_rejects_when_parent_canonicalizes_outside_root() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let tayf_root = tmp.path().join("tayf");
        std::fs::create_dir_all(&tayf_root).expect("mkdir tayf");
        let outside_dir = tmp.path().join("outside-config");
        std::fs::create_dir_all(&outside_dir).expect("mkdir outside");
        // Symlink ~/.config/tayf/profiles → /tmp/.../outside-config
        let profiles_link = tayf_root.join("profiles");
        std::os::unix::fs::symlink(&outside_dir, &profiles_link).expect("symlink dir");
        let dest = profiles_link.join("aws.toml");

        let err = check_safe_write_destination(&dest, &tayf_root)
            .expect_err("parent canonicalizing outside root must be rejected");
        assert!(
            err.to_lowercase().contains("outside"),
            "rejection reason must mention 'outside'; got: {err}"
        );
    }

    #[test]
    fn check_safe_write_destination_accepts_nonexistent_dest_when_parent_inside_root() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let tayf_root = tmp.path().join("tayf");
        std::fs::create_dir_all(tayf_root.join("profiles")).expect("mkdir");
        let dest = tayf_root.join("profiles").join("does-not-exist-yet.toml");

        check_safe_write_destination(&dest, &tayf_root)
            .expect("nonexistent dest is fine when parent is safe");
    }

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
            "last-write wins by RuleId key; first edit silently lost — documented behavior"
        );
    }

    #[test]
    fn integration_commit_save_with_general_theme_edit_persists_to_disk() {
        // Spec §7.2 I1.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.general.theme = Some(Some("light".to_owned()));
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_779_667_200);
        commit_save(&snap, &edits, now).expect("save");
        // Disk content updated.
        let disk_after = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(disk_after, "[general]\ntheme = \"light\"\n");
        // Backup contains pre-edit bytes.
        let backup = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .expect("backup exists");
        let backup_bytes = std::fs::read(backup.path()).unwrap();
        assert_eq!(backup_bytes, b"[general]\ntheme = \"dark\"\n");
    }

    #[test]
    fn integration_commit_save_preserves_header_comments() {
        // Spec §7.2 I2.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        let source = b"# Header comment\n# Two lines\n[general]\ntheme = \"dark\"\n";
        std::fs::write(&cfg_path, source).unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.general.theme = Some(Some("light".to_owned()));
        commit_save(&snap, &edits, SystemTime::now()).expect("save");
        let disk_after = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            disk_after.starts_with("# Header comment\n# Two lines\n"),
            "header preserved: {disk_after:?}"
        );
        assert!(disk_after.contains("theme = \"light\""), "theme updated");
    }

    #[test]
    fn integration_commit_save_preserves_rule_ordering() {
        // Spec §7.2 I3.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        let source =
            b"[[rules]]\nname = \"a\"\npattern = \"old_a\"\n\n[[rules]]\nname = \"b\"\npattern = \"keep_b\"\n";
        std::fs::write(&cfg_path, source).unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("a".to_owned()),
            RuleEdit {
                pattern: Some("new_a".to_owned()),
                styles: std::collections::HashMap::new(),
            },
        );
        commit_save(&snap, &edits, SystemTime::now()).expect("save");
        let disk_after = std::fs::read_to_string(&cfg_path).unwrap();
        let a_pos = disk_after.find("name = \"a\"").expect("a present");
        let b_pos = disk_after.find("name = \"b\"").expect("b present");
        assert!(a_pos < b_pos, "a must come before b; got disk: {disk_after:?}");
        assert!(disk_after.contains("new_a"), "a's pattern updated");
        assert!(disk_after.contains("keep_b"), "b's pattern untouched");
    }

    #[test]
    fn integration_commit_save_user_config_override_writes_stub_entry() {
        // Spec §7.2 I4 — the `o` keystroke shape: RuleEdit::default() insert.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.rules.insert(RuleId::UserConfig("uuid".to_owned()), RuleEdit::default());
        commit_save(&snap, &edits, SystemTime::now()).expect("save");
        let disk_after = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(disk_after.contains("[[rules]]"), "stub entry section added");
        assert!(disk_after.contains("name = \"uuid\""), "uuid name written");
        // Stub has only name; no pattern, no style appended.
        let rules_section_start = disk_after.find("[[rules]]").expect("rules section present");
        let rules_section = &disk_after[rules_section_start..];
        assert!(
            !rules_section.contains("pattern"),
            "stub must NOT have pattern key: {rules_section:?}"
        );
        assert!(
            !rules_section.contains("style"),
            "stub must NOT have style key: {rules_section:?}"
        );
    }

    #[test]
    fn integration_commit_save_reconcile_error_propagates_as_io_error_no_backup_written() {
        // Spec §7.2 I5. G8 refactor: reconcile is now a pre-flight in
        // commit_save (runs before commit_bytes touches the filesystem),
        // so a failing reconcile NEVER leaves an orphan backup — strictly
        // cleaner than the pre-G8 "8-step ceremony writes backup before
        // reconcile" sequencing which produced orphan files on failure.
        use crate::config_tui::edit::RuleId;
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        let source = b"[general]\ntheme = \"dark\"\n";
        std::fs::write(&cfg_path, source).unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.deleted.insert(RuleId::Builtin("uuid"));
        let err = commit_save(&snap, &edits, SystemTime::now()).expect_err("must error");
        assert_eq!(
            err.to_string(),
            "reconcile failed: unsupported deletion target: Builtin(\"uuid\") \
             (currently only `RuleId::UserConfig` deletion is supported; \
             other variants are reserved for future work)",
            "full error chain byte-pinned"
        );
        // G8 invariant: failing reconcile pre-flight writes NO backup.
        let backup_entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tayf-backup-"))
            .collect();
        assert_eq!(
            backup_entries.len(),
            0,
            "no backup file when reconcile fails pre-flight (G8 sequencing)"
        );
        // Source on disk unchanged (commit failed before any IO).
        let disk_after = std::fs::read(&cfg_path).unwrap();
        assert_eq!(disk_after, source as &[u8], "source unchanged on reconcile fail");
    }

    #[test]
    fn integration_commit_save_post_save_snapshot_parses_edits() {
        // Spec §7.2 I6.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.general.theme = Some(Some("light".to_owned()));
        let new_snap = commit_save(&snap, &edits, SystemTime::now()).expect("save");
        assert_eq!(new_snap.parsed.theme.as_deref(), Some("light"));
    }

    #[test]
    fn write_atomic_to_creates_file_with_expected_content() {
        // v0.6.1 §3.3: write_atomic_to wraps TmpFileGuard::persist for
        // string bodies. End-to-end happy path.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let target = tmp.path().join("config.toml");
        write_atomic_to(&target, "hello\nworld\n").expect("atomic write");
        let read = std::fs::read_to_string(&target).expect("read back");
        assert_eq!(read, "hello\nworld\n");
    }

    #[test]
    fn write_atomic_to_creates_missing_parent_directories() {
        // v0.6.1 §3.3: first-run init may target ~/.config/tayf which
        // may not yet exist. write_atomic_to must mkdir -p the parent.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let target = tmp.path().join("nested/dir/config.toml");
        assert!(!target.parent().unwrap().exists());
        write_atomic_to(&target, "ok").expect("atomic write with mkdir");
        assert!(target.exists(), "target written");
    }

    #[test]
    fn integration_duplicate_rule_name_on_disk_mutates_first_occurrence_only() {
        // Spec §7.2 I7 + §13.2 I14 fold — silent first-match contract.
        use crate::config_tui::edit::{RuleEdit, RuleId};
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        let source =
            b"[[rules]]\nname = \"x\"\npattern = \"A\"\n\n[[rules]]\nname = \"x\"\npattern = \"B\"\n";
        std::fs::write(&cfg_path, source).unwrap();
        let snap =
            crate::config_tui::snapshot::ConfigSnapshot::read_from_disk(Some(&cfg_path)).unwrap();
        let mut edits = crate::config_tui::edit::PendingEdits::default();
        edits.rules.insert(
            RuleId::UserConfig("x".to_owned()),
            RuleEdit { pattern: Some("NEW".to_owned()), styles: std::collections::HashMap::new() },
        );
        commit_save(&snap, &edits, SystemTime::now()).expect("save");
        let disk_after = std::fs::read_to_string(&cfg_path).unwrap();
        let new_pos = disk_after.find("NEW").expect("NEW present");
        let b_pos = disk_after.find("\"B\"").expect("B preserved");
        assert!(new_pos < b_pos, "first occurrence mutated, second preserved: {disk_after:?}");
        // Both x entries still have name = "x":
        assert_eq!(disk_after.matches("name = \"x\"").count(), 2);
        assert!(
            !disk_after.contains("pattern = \"A\""),
            "first entry's old pattern A must be gone: {disk_after:?}"
        );
    }
}
