//! `ConfigSnapshot` — disk read + SHA-256 + `toml_edit::DocumentMut` parse.
//!
//! Frozen view of disk config at last successful read. Used by the TUI
//! to detect concurrent manual edits (D1 conflict modal trigger) and to
//! reconstruct the typed `ParsedConfigView` overlay for live preview.

// reason: downstream Phase C tasks (C1b/C1c/C2*) consume these items;
// `expect` auto-fires a warning once the allow becomes unnecessary,
// so the suppression is self-removing as the rest of Phase C lands.
#![expect(dead_code, reason = "constructed and read by downstream Phase C tasks (C1b/C1c/C2*)")]

use std::path::{Path, PathBuf};

use crate::config::{Config, GeneralSection, UserRule};
use crate::error::Result;

/// Frozen view of disk config at last successful read.
#[derive(Debug)]
pub(crate) struct ConfigSnapshot {
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) source_hash: [u8; 32],
    pub(crate) raw_bytes: Vec<u8>,
    pub(crate) doc: toml_edit::DocumentMut,
    pub(crate) parsed: ParsedConfigView,
}

/// Lightweight typed view derived from `doc`. Reuses existing config types.
#[derive(Debug)]
pub(crate) struct ParsedConfigView {
    pub(crate) general: GeneralSection,
    pub(crate) rules: Vec<UserRule>,
    pub(crate) theme: Option<String>,
    pub(crate) profile: Option<String>,
}

impl ConfigSnapshot {
    /// Read the config file at `path`. Returns the empty-snapshot
    /// shape when `path` is `None` (no user config file exists).
    pub(crate) fn read_from_disk(path: Option<&Path>) -> Result<Self> {
        let Some(p) = path else {
            return Ok(Self::empty());
        };
        let raw_bytes = std::fs::read(p).map_err(|e| crate::error::Error::Config {
            path: p.display().to_string(),
            line: 0,
            message: format!("cannot read config: {e}"),
        })?;
        let source_hash = sha256(&raw_bytes);
        let raw_str = std::str::from_utf8(&raw_bytes).map_err(|e| crate::error::Error::Config {
            path: p.display().to_string(),
            line: 0,
            message: format!("config is not valid UTF-8: {e}"),
        })?;
        let doc: toml_edit::DocumentMut =
            raw_str.parse().map_err(|e: toml_edit::TomlError| crate::error::Error::Config {
                path: p.display().to_string(),
                line: 0,
                message: format!("toml_edit parse: {e}"),
            })?;
        let cfg: Config = crate::config::parse(&p.display().to_string(), raw_str)?;
        let parsed = ParsedConfigView {
            theme: cfg.general.theme.clone(),
            profile: cfg.general.profile.clone(),
            general: cfg.general,
            rules: cfg.rules,
        };
        Ok(Self { source_path: Some(p.to_path_buf()), source_hash, raw_bytes, doc, parsed })
    }

    /// Synthetic snapshot for the no-config-file case. `doc` is an
    /// empty `DocumentMut`; SHA256 of empty bytes is the all-zeros-but-known hash.
    pub(crate) fn empty() -> Self {
        let raw_bytes = Vec::new();
        let source_hash = sha256(&raw_bytes);
        Self {
            source_path: None,
            source_hash,
            raw_bytes,
            doc: toml_edit::DocumentMut::new(),
            parsed: ParsedConfigView {
                general: GeneralSection::default(),
                rules: Vec::new(),
                theme: None,
                profile: None,
            },
        }
    }
}

/// FIPS 180-4 §4.2.2 round constants — first 32 bits of the fractional
/// parts of the cube roots of the first 64 primes.
#[rustfmt::skip]
const SHA256_K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4, 0xab1c_5ed5,
    0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174,
    0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
    0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967,
    0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85,
    0xa2bf_e8a1, 0xa81a_664b, 0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
    0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
];

/// FIPS 180-4 §5.3.3 initial hash value — first 32 bits of the fractional
/// parts of the square roots of the first 8 primes.
#[rustfmt::skip]
const SHA256_H0: [u32; 8] = [
    0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
    0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
];

/// Hand-rolled SHA-256 (no new dep). Uses the standard FIPS 180-4
/// algorithm. ~60 LOC, lifted from public-domain reference. We avoid
/// pulling `sha2` because file hashing here is a non-hot-path
/// scratch use and adding a crate for one call site fails the
/// memory `feedback_dependency_minimalism` bar.
// reason: SHA-256 working variables are named `a..h` and `w` per the
// FIPS 180-4 spec (§6.2.2). Renaming them away from the spec letters
// would make the code harder to audit against the standard.
#[allow(clippy::many_single_char_names, reason = "FIPS 180-4 spec letters")]
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = SHA256_H0;
    let mut padded = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, b) in chunk.chunks_exact(4).enumerate().take(16) {
            w[i] = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 =
                hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, &word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{sha256, ConfigSnapshot};

    #[test]
    fn sha256_known_empty_answer() {
        // FIPS 180-4 reference: SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = sha256(b"");
        let hex = h.iter().fold(String::with_capacity(64), |mut acc, b| {
            use std::fmt::Write as _;
            write!(acc, "{b:02x}").expect("writing to String never fails");
            acc
        });
        assert_eq!(hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn sha256_known_abc_answer() {
        // FIPS 180-4 reference: SHA-256("abc") =
        // ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let h = sha256(b"abc");
        let hex = h.iter().fold(String::with_capacity(64), |mut acc, b| {
            use std::fmt::Write as _;
            write!(acc, "{b:02x}").expect("writing to String never fails");
            acc
        });
        assert_eq!(hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn read_from_disk_roundtrip_yields_same_hash() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[general]\ntheme = \"dark\"\n").unwrap();
        let snap_a = ConfigSnapshot::read_from_disk(Some(&cfg_path)).expect("snap a");
        let snap_b = ConfigSnapshot::read_from_disk(Some(&cfg_path)).expect("snap b");
        assert_eq!(snap_a.source_hash, snap_b.source_hash, "two reads must yield identical hash");
        assert_eq!(snap_a.parsed.theme.as_deref(), Some("dark"));
        assert_eq!(snap_a.parsed.profile, None);
    }

    #[test]
    fn read_from_disk_missing_path_returns_empty_snapshot() {
        let snap = ConfigSnapshot::read_from_disk(None).expect("empty snap");
        assert!(snap.source_path.is_none());
        assert!(snap.raw_bytes.is_empty());
        assert!(snap.parsed.rules.is_empty());
    }

    #[test]
    fn read_from_disk_malformed_toml_returns_typed_error() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let cfg_path = tmp.path().join("config.toml");
        std::fs::write(&cfg_path, b"[broken").unwrap();
        let err = ConfigSnapshot::read_from_disk(Some(&cfg_path)).expect_err("must error");
        // Must be Error::Config (not Tty) — surfaces toml_edit's diagnostic.
        assert!(matches!(err, crate::error::Error::Config { .. }), "got: {err:?}");
    }
}
