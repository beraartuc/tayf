# Security Policy

## Reporting a Vulnerability

Report security issues privately to **bera@korp.com.tr**. Do not open a public
issue for a suspected vulnerability. We aim to acknowledge reports within 7 days
and to ship a fix or mitigation before any public disclosure.

## Supported Versions

tayf is pre-1.0; only the latest released minor version receives security fixes.

| Version | Supported |
| ------- | --------- |
| 0.9.x   | ✅        |
| < 0.9   | ❌        |

## Threat Model

tayf runs in the user's terminal, spawns their shell, manipulates their TTY, and
sits in the I/O path of every command. PTY output is treated as attacker-
controlled (e.g. `tail` of untrusted logs, `ssh` to a malicious host). The byte
path is hardened against ReDoS, escape-sequence injection, memory exhaustion,
and terminal-state corruption. See the project design docs for details.

## Non-Goals (explicitly out of scope)

- **Sandboxing (seccomp/landlock).** tayf is a transparent pass-through; sandboxing
  the child shell would defeat the tool's purpose. tayf assumes a single-user
  threat model — an attacker who can write to `~/.config/tayf/` can already run
  arbitrary commands as that user.
- **Windows.** tayf is Unix-only (PTY + termios + POSIX signals).
- **Multi-user isolation.** Out of scope per the single-user threat model above.
