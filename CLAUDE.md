# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RuSSH is a cross-platform SSH/SFTP desktop client built with Tauri 2 (Rust), React 19, TypeScript 5.9, Tailwind CSS 4, and Vite 7. The Rust runtime in `src-tauri/` owns SSH/SFTP, local terminals, SQLite persistence, themes, port forwarding, and update checks; the React renderer in `src/renderer/src/` is the unchanged workbench UI. It ships for Windows, macOS, and Linux.

## Common Commands

```bash
npm run dev                  # Start Tauri dev (Vite on 127.0.0.1:5173 + cargo run)
npm run build                # Typecheck + build frontend assets only
npm run tauri:build          # Full Tauri build (Rust + frontend + bundles)
npm run typecheck            # Run both node and web TypeScript checks
npm run typecheck:node       # tsc for config files + src/shared
npm run typecheck:web        # tsc for renderer + shared
npm run test                 # Vitest (jsdom, globals: true)
npx vitest run <file>        # Run a single test file
npx vitest run -t "<name>"   # Filter tests by name pattern
npm run lint                 # ESLint (has pre-existing failures - not fully green)
npm run format               # Prettier (single quotes, no semicolons, 100 char width)
npm run dist                 # Tauri build for the current platform
npm run dist:win             # Windows NSIS
npm run dist:mac             # macOS APP + DMG
npm run dist:linux           # Linux AppImage + DEB + RPM
```

`make` targets (`check`, `build`, `bundle-*`) wrap the same Tauri/Cargo commands.

## Test Infrastructure

Vitest runs with `globals: true` (no imports needed for `describe`/`it`/`expect`), `jsdom` environment, and a setup file at `test/renderer/helpers/setup.ts` that stubs `ResizeObserver` and `scrollIntoView`. Tests live in `test/renderer/` or `test/shared/` with `.test.ts` or `.test.tsx` extensions.

## Architecture

### Tauri Runtime (Rust)

- `src-tauri/src/main.rs` is a three-line entry that calls `russh_tauri_lib::run()`.
- `src-tauri/src/lib.rs` holds the entire backend: `RuntimeState` (SQLite connection, SSH sessions, SFTP streams, local PTYs, port-forward rules, transfer cancellations, trust prompts, logs), Tauri commands, event emitters, tray menu, and the application builder.
- `src-tauri/capabilities/default.json` scopes permissions to the `main` window (`core:default`, window dragging, dialog plugin).
- `src-tauri/compatibility.js` is injected at document start by `CompatibilityPlugin`; it defines `window.russhApi` on top of Tauri `invoke`/events so the renderer contract is unchanged.
- `src-tauri/gen/schemas/` contains generated capability schemas.

### Frontend Contract

The typed contract is unchanged from the renderer's perspective:

1. `src/shared/types.ts` — domain types
2. `src/shared/api.ts` — `RuSSHApi` interface (the contract consumed by the renderer)
3. `src/shared/ipc-channels.ts` — typed push event channels
4. `src/shared/tauri-bindings.ts` — Specta-generated command bindings (do not edit by hand)

### Renderer Feature API Gateway

Components must NOT access `window.russhApi` directly — an ESLint rule enforces this. Instead, use per-domain clients from `src/renderer/src/features/*/api/`. The proxy-based client is in `features/shared/api/russh-client.ts`. React Query keys are centralized in `features/shared/query-keys.ts`.

### Key Runtime Areas (Rust)

- SSH sessions, SFTP, and port forwarding — `src-tauri/src/lib.rs` session commands and workers
- Local terminals — `portable-pty` process manager
- Persistence — `rusqlite` (bundled) database at the Tauri app-data path
- Themes — bundled JSON theme packs plus user ZIP imports
- Updates — GitHub Releases manual check; signed Tauri updater install is not configured
- WebDAV — backup upload, listing, restore with SQLite-header validation

### State Management (Renderer)

Zustand stores in `src/renderer/src/store/`: `workbench-store.ts` (persisted UI chrome), `sessions-store.ts` (session tabs and connection phases), `local-terminals-store.ts`, `update-dialog-store.ts`.

### Workbench Document Model

`WorkbenchDocument` is a tagged union with 7 document kinds: server-editor, session-editor, sftp-file-editor, local-terminal-editor, settings-editor, updates-editor, terminal-welcome. Session and local-terminal editors use a keep-mounted strategy (hidden via visibility, not unmounted) to avoid xterm remounting costs. Terminal panes use xterm with addons (fit, search, webgl, image, web-links). The SFTP file editor uses Monaco Editor with language detection based on file extension.

## Path Aliases

- `@/*` and `@renderer/*` → `src/renderer/src/`
- `@shared/*` → `src/shared/`
- `@test/*` → `test/` (Vitest only)

## Key Constraints

- **Secrets live in SQLite**: server password/passphrases, credential vault secrets, and private keys are stored in the database, not as file paths.
- **Session identity**: `sessionId` flows from renderer provisional tab through to Rust — do not break this chain.
- **Jump server is single-hop only** — nested chains are explicitly unsupported.
- **Port forwarding rules are in-memory only** (no DB persistence).
- **Resource monitoring is Linux-only** via `/proc/*` and `df`.
- **WebDAV restore triggers app relaunch** — renderer cannot safely continue after restore.
- **`localTerminalShell` is platform-normalized** on both read and write.
- **`npm run lint` is not fully green** — pre-existing React hooks and fast refresh violations exist. Do not introduce new lint violations.

## Conventions

- Conventional commits enforced via husky + commitlint (feat, fix, perf, security, refactor, docs, style, test, build, ci, chore, revert)
- Prettier: single quotes, no semicolons, 100 char print width
- 2-space indentation (EditorConfig)
- UI components use shadcn/ui (new-york style) with Radix UI primitives and Lucide icons
- i18n supports zh-CN and en-US via i18next/react-i18next
- Tests live in `test/` (not alongside source), organized by `test/renderer/` and `test/shared/`

## Environment Requirements

- Node.js 22 LTS and npm 10+ recommended.
- Rust stable toolchain with `cargo`; Linux also needs GTK/WebKitGTK development packages.
- Verification order when validating changes: `npm run typecheck` → `npm run test`. `npm run lint` is not a gate (see Key Constraints).

## Existing Documentation

- `AGENTS.md` — detailed engineering snapshot with architecture, data flows, and boundaries
- `TAURI_MIGRATION.md` — migration status log
- `docs/theme-dev.md` — theme development guide
- `CHANGELOG.md` — auto-generated conventional changelog
