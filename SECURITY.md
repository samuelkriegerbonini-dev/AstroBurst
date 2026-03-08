# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.3.x   | :white_check_mark: |
| 0.2.x   | :white_check_mark: |
| 0.1.x   | :x:                |

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

AstroBurst processes astronomical data files (FITS and ASDF formats) which can contain arbitrary binary data. Security considerations include:

- **FITS parsing** -- Malformed headers, data segments, or crafted NAXIS/BITPIX values that could cause out-of-bounds reads via memory mapping
- **ASDF parsing** -- Malformed YAML trees, invalid block headers, decompression bombs (zlib/bzip2/lz4 blocks with extreme compression ratios), and crafted ndarray shapes that could cause excessive memory allocation
- **Memory mapping** -- Large file handling via `memmap2` with sequential and random access patterns
- **ZIP extraction** -- Nested ZIP traversal (capped at depth 4) for FITS-in-ZIP workflows; path traversal and zip bombs
- **Decompression** -- ASDF blocks support zlib (`flate2`), bzip2, and lz4 (`lz4_flex`) decompression; crafted blocks could trigger excessive memory use
- **Network requests** -- Astrometry.net API integration (when enabled via user-provided API key)
- **File system access** -- Read/write operations scoped by Tauri capabilities; output directories created on demand
- **Configuration storage** -- API keys stored in plaintext in platform config directory (`~/.config/astroburst/`)
