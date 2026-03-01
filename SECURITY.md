# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in AstroBurst, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, email **samuel.kriegerb@gmail.com** with:

- Description of the vulnerability
- Steps to reproduce
- Impact assessment
- Suggested fix (if any)

You can expect an acknowledgment within 48 hours and a resolution timeline within 7 days.

## Scope

AstroBurst processes astronomical data files (FITS format) which can contain arbitrary binary data. Security considerations include:

- **FITS parsing** — Malformed headers or data segments
- **Memory mapping** — Large file handling via `memmap2`
- **Network requests** — Astrometry.net API integration (when enabled)
- **File system access** — Read/write operations scoped by Tauri capabilities
