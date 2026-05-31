#!/usr/bin/env bash
# Deterministic demo content for recording a tayf asciinema cast.
#
# tayf wraps your whole shell, so the demo is: start a recording, launch tayf,
# run this script inside the tayf session, then exit. tayf colorizes the output
# below in real time. See docs/demo/README.md for the full recording recipe.
#
# The lines here are crafted to exercise the built-in patterns (IPs, log levels,
# timestamps, durations, permissions, URLs, UUIDs, FQDNs, file names). The exact
# colors come from tayf's defaults at the version you are recording — this script
# is intentionally color-agnostic.

set -euo pipefail

type() { printf '%s\n' "$1"; sleep "${2:-0.6}"; }

type '$ ls -l /var/log'
type 'drwxr-xr-x  2 root root 4096 2026-05-31T09:12:04Z .'
type '-rw-r--r--  1 root root 1240 2026-05-31T09:12:04Z syslog'
type ''
type '$ tail -f /var/log/app.log'
type '2026-05-31T09:12:05Z  INFO  server listening on https://api.example.com'
type '2026-05-31T09:12:06Z  WARN  slow query took 1284.51 ms from 192.168.1.42'
type '2026-05-31T09:12:07Z  ERROR auth failed for request 550e8400-e29b-41d4-a716-446655440000'
type '2026-05-31T09:12:07Z  INFO  GET /healthz 200 in 3.21 ms'
type '2026-05-31T09:12:08Z  DEBUG upstream fe80::1 reset after 45.0 us'
type ''
type '$ curl -s https://example.com/status'
type '{ "ok": true, "peer": "10.0.0.7", "host": "db-01.internal.example.com" }'
