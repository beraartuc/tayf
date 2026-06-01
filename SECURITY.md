# Security Policy

## Reporting a Vulnerability

Report security issues privately to **bera@korp.com.tr**. Do not open a public
issue for a suspected vulnerability. We aim to acknowledge reports within 7 days
and to ship a fix or mitigation before any public disclosure.

## Supported Versions

tayf is pre-1.0; only the latest released minor version receives security fixes.

| Version | Supported |
| ------- | --------- |
| 0.11.x  | ✅        |
| < 0.11  | ❌        |

## Threat Model

tayf runs in the user's terminal, spawns their shell, manipulates their TTY, and
sits in the I/O path of every command. PTY output is treated as attacker-
controlled (e.g. `tail` of untrusted logs, `ssh` to a malicious host). The byte
path is hardened against ReDoS, escape-sequence injection, memory exhaustion,
and terminal-state corruption. See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for details.

A known, benign race exists in signal teardown: between reaping the child and
stopping the signal thread, a forwarded signal can call `killpg` on a just-reaped
process group. This is harmless — `killpg` on an empty group returns `ESRCH`
(swallowed), and mis-targeting is not reachable from the attacker-controlled PTY
output stream. It is the same race every PTY wrapper (tmux, script, sudo)
accepts, closable only with Linux `pidfd`.

## Releases

Published crates.io versions are immutable: a release can be **yanked** (which
stops new dependents from selecting it) but never deleted, and a version number
is never reused. A bad release is remediated by yanking it and publishing a fixed
patch version. Release binaries carry keyless Sigstore build provenance — verify
with `gh attestation verify <binary> --repo beraartuc/tayf`.

## Non-Goals (explicitly out of scope)

- **Sandboxing (seccomp/landlock).** tayf is a transparent pass-through; sandboxing
  the child shell would defeat the tool's purpose. tayf assumes a single-user
  threat model — an attacker who can write to `~/.config/tayf/` can already run
  arbitrary commands as that user.
- **Windows.** tayf is Unix-only (PTY + termios + POSIX signals).
- **Multi-user isolation.** Out of scope per the single-user threat model above.
