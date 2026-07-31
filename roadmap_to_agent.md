# Roadmap: Evolving AstroBurst for AI-Agent Integration

This document outlines the strategic roadmap for transforming AstroBurst from a tightly-coupled desktop application into an AI-agent-friendly ecosystem. The final goal is to establish a purely headless backend capable of intensive astronomical image processing, paired with a distinct frontend repository/process, enabling an AI agent to programmatically control the backend without manual user-interface interactions.

## Current State (Codebase-Grounded)

Before planning the migration, it is worth correcting a common assumption: the core engine is **not** heavily interwoven with Tauri. A pass over `src-tauri/src` shows the decoupling is already mostly done.

- **`core`: 0 Tauri references. `math`: 0 Tauri references.** Both are already pure Rust and need no detangling.
- **`infra`: a single coupling point**, confined to `src-tauri/src/infra/progress.rs`, which uses `tauri::Emitter` and `tauri::AppHandle` to push progress events. This is the only "deep" coupling in the core trio.
- **`cmd`: the real coupling lives here** — ~76 references to `tauri::State` / `AppHandle` / `Window` across the handlers, exposed through ~15 `#[tauri::command]` entry points. This is the genuine surface area to refactor.
- **Backend is a single crate** (~24k LOC, one `Cargo.toml`), not yet a workspace. Library extraction is therefore mechanical (module moves + a workspace split), not a deep rewrite.
- **Frontend already abstracts IPC** behind one thin file, `src/infrastructure/tauri/client.ts` (`typedInvoke<T>`, `getPreviewUrl`, an `isTauri()` guard, and a non-Tauri stub). The migration seam already exists.
- **Four native plugins are in play:** `@tauri-apps/plugin-{dialog,fs,opener,shell}`. These provide OS-native file selection, filesystem access, file/URL opening, and shell access — capabilities a headless server cannot expose identically, so they need an explicit migration story (see Cross-Cutting Concerns).

**Implication:** the heavy lift is not "untangling the engine" — it is (1) abstracting the progress emitter, (2) extracting the `cmd` service layer, and (3) replacing native-plugin UX on the frontend. Phase 1 is closer to done than it appears.

## The End State

- **Pure Headless Backend (Rust):** A standalone high-performance Rust service (REST/gRPC API and/or CLI) responsible for FITS I/O, intensive memory-mapped analysis, rendering, alignment, stacking, and all core logic.
- **Distinct Frontend Repository/Process:** The React/Web application is fully separated from the Rust core, communicating solely through standard network protocols (HTTP/WebSocket) rather than native IPC bindings.
- **AI-Agent Interface:** An AI agent submits declarative processing pipelines (e.g., "Load M51, extract background, apply SHO stretch, save to PNG"). The backend executes autonomously and returns results (JSON, generated images).
- **Tauri Elimination:** Tauri, which currently binds the Rust and React sides into a single executable, is fully removed.

---

## Phase 1: Decoupling and Library Extraction

The real coupling is concentrated in `cmd` (handlers) and `infra/progress.rs` (event emission), not in `core`/`math`.

1. **Abstract the progress emitter.**
    - Replace the `tauri::Emitter` / `AppHandle` usage in `infra/progress.rs` with a transport-agnostic sink (a `trait ProgressSink` or an `mpsc`/broadcast channel).
    - Tauri, WebSocket/SSE, and CLI stdout each implement or consume that sink. This single change removes the last Tauri reference from the core trio.
2. **Extract Core Library (`astroburst-core`).**
    - Split the single crate into a workspace; move `core`, `math`, and the now-Tauri-free `infra` into a pure library crate.
    - Ensure `cache`/`infra` derive paths from explicit configuration or `dirs`, never from Tauri's app-directory context.
3. **Abstract Command Handlers.**
    - Rewrite the ~15 command handlers in `src-tauri/src/cmd` as protocol-agnostic service functions that accept plain Rust structs and return `Result<T, E>`.
    - The existing Tauri handlers become thin wrappers over the service layer, keeping the desktop app working throughout the refactor.

## Phase 2: Building the Headless Backend

Create new executable targets that consume `astroburst-core`. **Recommended ordering: ship the CLI first** — it delivers agent-usable value without depending on the HTTP server or the frontend split, and it validates the declarative-pipeline design early at the lowest risk.

1. **CLI / Declarative Pipeline Engine (do this first).**
    - Implement a CLI (`clap`) that consumes a JSON/YAML manifest describing a full pipeline.
    - Example: `astroburst-cli run pipeline.json --output ./results`
2. **API Server Implementation.**
    - Implement an HTTP/WebSocket server (`axum` or `actix-web`).
    - Expose the protocol-agnostic service functions (Phase 1) as REST endpoints or WebSocket RPC.
    - Example endpoints: `/api/io/process`, `/api/compose/align`, `/api/stack/calibrate`.
    - Bridge the `ProgressSink` from Phase 1 to WebSocket/SSE for real-time progress.
3. **State Management.**
    - Replace Tauri-managed application state with an explicit strategy for the ORIG/KEY dual cache (see Cross-Cutting Concerns) — this is the deepest design decision in the migration, not an afterthought.

## Phase 3: Frontend Migration

With the headless backend operational, transition the React frontend off Tauri IPC.

1. **Repository Split.**
    - Move the React codebase (`src/`) into a separate repository or a clearly separated monorepo package.
    - Remove `@tauri-apps/*` dependencies from `package.json`.
2. **API Client Integration.**
    - Promote `src/infrastructure/tauri/client.ts` to `src/infrastructure/api/client.ts`, keeping the `typedInvoke<T>` signature but backing it with `fetch`/WebSocket instead of Tauri `invoke`. Because the seam already exists, call sites stay unchanged.
3. **Native Plugin Replacement (the real frontend work).**
    - `plugin-dialog` / `plugin-fs`: replace native file pickers with browser uploads or a server-side path browser; decide how the backend receives input paths.
    - `plugin-opener` / `plugin-shell`: re-implement or drop; these have no direct headless equivalent.
    - Replace Tauri's `asset://` protocol (and `convertFileSrc` in `client.ts`) with HTTP static serving from the backend.
4. **Desktop App Alternative (Optional).**
    - If a unified desktop deliverable is still required, wrap the standalone frontend in Electron or a WebView shell that spawns and manages the Rust backend process.

## Phase 4: Designing the AI-Agent Interface

Optimize the backend API for programmatic reasoning and execution by LLMs or specialized agents.

1. **Agent-Friendly Schema.**
    - Publish OpenAPI/Swagger specs for the API so agents can auto-generate tools and understand endpoint signatures.
2. **Pipeline Manifest Definition.**
    - Formalize the JSON/YAML schema for declarative pipelines. An agent should be able to construct a single payload such as:
      ```json
      {
        "inputs": ["path/to/R.fits", "path/to/G.fits", "path/to/B.fits"],
        "steps": [
          { "action": "align", "method": "phase_correlation" },
          { "action": "background_extraction", "degree": 4 },
          { "action": "stretch", "type": "ghs", "params": { "symmetry_point": 0.5 } }
        ],
        "exports": ["png", "fits"]
      }
      ```
3. **Robust Error Handling & Feedback.**
    - Return rich, deterministic errors (e.g., "Alignment failed: insufficient star matches in R.fits") so an agent can self-correct and retry gracefully.
4. **Headless Output Artifacts.**
    - Cleanly return generated artifacts (file paths, base64 thumbnails, statistics, logs) for immediate ingestion into the agent's context.

---

## Cross-Cutting Concerns

These span all phases and must be decided early; they are the actual risk centers.

- **State & cache for large images (ORIG/KEY).** The dual cache holds gigabyte-scale memory-mapped buffers. A stateless HTTP model cannot keep these alive across requests for free. Choose explicitly: session handles with TTL eviction, or a content-addressed artifact store the agent references by id. This decision shapes the entire API.
- **Rendering / GPU.** The backend uses wgpu (`src-tauri/src/shaders`, `GpuRenderer`). Headless rendering needs an offscreen GPU context or a guaranteed CPU fallback path; servers frequently run without a display/GPU. Plan for both.
- **Security & sandboxing.** A REST API performing FITS I/O over arbitrary paths and executing pipelines is an arbitrary-file-read / path-traversal surface. For an agent-driven backend, define authentication, a path allow-list / jailed working directory, and resource limits before exposing it.
- **Real-time progress.** The existing event system (`infra/progress.rs`) maps naturally onto the `ProgressSink` abstraction (Phase 1) feeding WebSocket/SSE (Phase 2). Treat it as one continuous thread of work rather than two unrelated mentions.

## Suggested Sequencing

1. Abstract `ProgressSink` + extract `astroburst-core` (Phase 1).
2. Ship the CLI pipeline engine (Phase 2.1) — earliest agent-usable value, lowest risk.
3. Decide the state/cache model, then build the HTTP/WebSocket server (Phase 2.2–2.3).
4. Migrate the frontend, focusing on native-plugin replacement (Phase 3).
5. Harden into the agent interface: OpenAPI, manifest schema, security (Phase 4 + Cross-Cutting).

---

**Summary:** AstroBurst's engine is already largely Tauri-free; the migration is less about untangling `core`/`math` and more about abstracting the progress emitter, extracting the `cmd` service layer, replacing native-plugin UX, and answering the hard questions around large-image state, headless GPU, and security. Sequenced CLI-first, the project can deliver agent-usable capability early while building toward a fully headless, automated engine that supports AI-driven discovery over massive astronomical datasets.

---

*Original roadmap contributed by [@leejjoon](https://github.com/leejjoon) (applied manually from their PR — thank you!). Revised with codebase-grounded current-state analysis, cross-cutting risk notes, and a CLI-first sequencing recommendation.*
