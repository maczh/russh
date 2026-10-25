# AGENTS.md

Cross-platform SSH/SFTP desktop client. Tauri 2 (Rust) + React 19 + TypeScript 5 + Vite 7 + Tailwind 4. The Rust runtime owns SSH/SFTP, PTYs, SQLite, themes, port forwarding, and update checks.

> **Child AGENTS.md**: `src/renderer/src/`, `test/` — read domain-specific docs there.

## STRUCTURE

```
src-tauri/          # Tauri 2 / Rust backend — see src-tauri/README.md
  src/lib.rs        # Entire backend: RuntimeState, commands, events, tray, builder
  src/main.rs       # Thin entry: russh_tauri_lib::run()
  compatibility.js  # Document-start adapter that defines window.russhApi
  capabilities/     # Tauri permission scopes (main window only)
  gen/schemas/      # Generated capability schemas
  tauri.conf.json   # Window, CSP, dev URL, bundling, icons
src/renderer/       # React workbench (index.html + src/)
src/shared/         # Shared models + API contract (Zod, RuSSHApi, Tauri Specta bindings)
themes/builtin/     # VSCode-style JSON theme packs (loaded by Rust)
test/               # Mirror-tree test infra — see child AGENTS.md
scripts/            # tauri-dev.mjs — starts Vite then cargo run
build/              # Bundle icons referenced by tauri.conf.json
docs/               # Theme development guide + historical design notes
```

**Non-obvious**: `src/renderer/src/` double nesting is the Vite root convention. No `pages/` dir — the entire app is a single workbench shell. There is no Node-based host process or preload directory; the renderer talks to Rust through `window.russhApi`, which is created by `src-tauri/compatibility.js`.

## WHERE TO LOOK

| Task                     | Location                                                                                                                                        | Notes                                            |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| Add new Rust command     | `src-tauri/src/lib.rs` command + `collect_commands!` → regenerate `src/shared/tauri-bindings.ts` → expose in `compatibility.js` if renderer needs it | Cross-cutting (see below)                        |
| Modify session lifecycle | `src-tauri/src/lib.rs` (session commands/workers) + `renderer/store/sessions-store.ts` + `renderer/components/workbench/workbench-context.tsx`    | Emits `sessions:state` / `sessions:error` / `sessions:data` |
| Change theme             | `src/shared/themes.ts` → theme commands in `lib.rs` → `themes/builtin/<pack>/themes/*.json` → `renderer/lib/theme.ts`                            | Theme packs are JSON, not CSS                    |
| Add server field         | `src-tauri/src/lib.rs` SQLite schema/mapping + `src/shared/validation.ts` (`ServerUpsertInput`/`serverSchema`) + `src/shared/types.ts`            | Also update the servers command payloads         |
| Add SFTP feature         | `src-tauri/src/lib.rs` SFTP commands + `renderer/features/sftp/api` + `workbench-sftp-*` components                                             | SFTP editor streaming is text-only               |
| Work on UI layout        | `renderer/components/workbench/`                                                                                                                | Keep-mounted for session/local-terminal editors  |
| Write renderer tests     | `test/renderer/` + `test/renderer/helpers/create-russh-api`                                                                                     | Never co-locate tests in src/                    |

## Commands

```bash
npm run dev              # Vite (127.0.0.1:5173) + cargo run — full app
npm run build            # typecheck + frontend build (no Rust)
npm run tauri:build      # Full Tauri build with bundles
npm run test             # All Vitest tests
npm run typecheck        # Both Node + Web tsc projects
npm run lint             # KNOWN to have historical failures — not a gate
npm run format           # Prettier
npm run dist:win         # Platform bundles via Tauri CLI
npm run dist:mac
npm run dist:linux

npx vitest run test/path/to/file.test.ts      # Single test file
npx vitest run -t "pattern name"              # Filter by test name

make check               # typecheck + cargo fmt --check + cargo check
make build               # Release Rust binary
make bundle-<platform>   # Tauri bundles for one platform
```

**Verification order**: `npm run typecheck` → `npm run test` (lint is not a gate)

## Path Aliases

```
@/          → src/renderer/src/
@renderer/  → src/renderer/src/
@shared/    → src/shared/
@test/      → test/  (Vitest only)
```

Use these aliases in imports; both src and tests consume them.

## Architecture

**Process boundary**: Rust (`src-tauri/`) ↔ WebView renderer (`src/renderer/src/`). Shared types/contract live in `src/shared/`.

- `src-tauri/src/main.rs` is a thin entry; all assembly is in `russh_tauri_lib::run()` in `lib.rs`.
- `RuntimeState` (managed by Tauri) holds the SQLite connection, SSH sessions, SFTP streams, local PTYs, port-forward rules, pending trust prompts, and log buffers.
- `src-tauri/compatibility.js` is injected as an initialization script and exposes the `RuSSHApi` surface on `window.russhApi`, translating calls into Tauri `invoke` calls and event subscriptions.
- `src/shared/tauri-bindings.ts` is generated by Tauri Specta — **do not edit manually**.
- `src/renderer/src/features/*/api` is the ONLY approved entry point for renderer code to call `window.russhApi`. ESLint enforces this: `window.russhApi` is banned in `src/renderer/src/**` except in `features/shared/api/`.
- `src/renderer/src/features/shared/query-keys.ts` centralizes all React Query keys. Never hardcode new keys.
- Theme definitions live in `themes/builtin/`, bundled by `tauri.conf.json` and loaded by Rust.

## Testing

- Tests live in **`test/`**, mirroring the `src/` structure — they are **NOT co-located** with source files.
- File pattern: `test/**/*.test.{ts,tsx}`. Setup file: `test/renderer/helpers/setup.ts`.
- Environment: `jsdom` with `globals: true` (no explicit imports of describe/it/expect needed).
- Renderer tests mock `window.russhApi` via `test/renderer/helpers/create-russh-api.ts`.
- Rust unit tests live next to the code in `src-tauri/src/lib.rs` and run via `cargo test --manifest-path src-tauri/Cargo.toml`.

## i18n

- Renderer strings: `src/renderer/src/i18n/resources/{en-US,zh-CN}/` (namespace-per-file).
- Adding a new i18n key requires updating **both** locale directories and the type definitions they export.

## Tauri Runtime Notes

- Rendering uses the platform WebView: WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux (dev packages required).
- The CSP in both `src/renderer/index.html` and `tauri.conf.json` allows `ipc://localhost`, `http://ipc.localhost`, inline styles, `data:`/`blob:` images, and `wasm-unsafe-eval`.
- Permissions are intentionally minimal (`core:default`, window dragging, dialog). Extend `src-tauri/capabilities/default.json` only alongside reviewed commands.
- Frontend dev requires the Vite server to be reachable on `127.0.0.1:5173`; `scripts/tauri-dev.mjs` blocks until it is listening before starting Cargo.

## Code Style

- **Prettier**: `singleQuote: true`, `semi: false`, `printWidth: 100`, `trailingComma: none`.
- **TypeScript**: strict mode, no `as any` / `@ts-ignore` allowed.
- **Commit format**: conventional commits enforced by commitlint (`feat`, `fix`, `perf`, `security`, `refactor`, `docs`, `style`, `test`, `build`, `ci`, `chore`, `revert`). Header max 120 chars. Pre-commit hook is empty (no lint-on-commit).
- **shadcn/ui** components live under `src/renderer/src/components/ui/` — do not add `react-refresh` exports there (eslint disabled intentionally).

## Critical Constraints

These are the facts an agent is most likely to get wrong:

- **ESLint blocks `window.russhApi`** in all `src/renderer/src/**` except `src/renderer/src/features/shared/api/**`. Always go through `features/*/api`.
- **Credential model** — server password/passphrase, vault secrets, and private keys all live in the SQLite database managed by Rust.
- **Jump server is single-hop only**. Nested jump chains are explicitly rejected.
- **Auto-update**: release metadata is fetched from GitHub Releases; installing an update requires a signed Tauri updater endpoint that is not yet configured.
- **Resource monitoring is Linux-only** (`/proc/stat`, `/proc/meminfo`, etc.).
- **Port forwarding is session-scoped memory only** — no DB persistence.
- **`session-editor` and `local-terminal-editor` use keep-mounted strategy** (visible via visibility toggle, not conditional render) to avoid xterm re-initialization on tab switch. Do not change this.
- **SFTP remote file editing is text-only** — editor streaming is chunked over Rust events; no binary/large-file editor support.
- **Restore (backup) triggers `system.relaunch`** — do not assume the app continues running after restore.
- **`npm run lint` has known repository-level failures** (React hooks, fast refresh). Do not fail a workflow because of lint; only fail on new lint errors introduced by your change.

## Cross-Cutting Changes

Changing any domain typically requires touching these layers together:

- **Rust command change**: `src-tauri/src/lib.rs` (`#[tauri::command]` + `collect_commands!`) → regenerate `src/shared/tauri-bindings.ts` → `src-tauri/compatibility.js` (`window.russhApi` shape) → keep `src/shared/api.ts` unchanged for renderer parity.
- **Session identity / phase**: `src-tauri/src/lib.rs` + `src/renderer/src/store/sessions-store.ts` + `src/renderer/src/components/workbench/workbench-context.tsx`.
- **Theme change**: `src/shared/themes.ts` → theme commands in `src-tauri/src/lib.rs` → `themes/builtin/<pack>/themes/*.json` → `src/renderer/src/lib/theme.ts`.
- **SFTP / remote editing**: Rust SFTP stream commands in `lib.rs` + `src/renderer/src/features/sftp/api/*` + `src/renderer/src/components/workbench/workbench-sftp-file-*-editor.tsx`.
- **Any change to the servers table schema**: update the Rust schema/mapping in `src-tauri/src/lib.rs` and `src/shared/validation.ts` (`ServerUpsertInput`/`serverSchema`).

## Test Mock for `window.russhApi`

When writing renderer tests that call the bridge:

```ts
import { createRuSSHApi } from '@test/helpers/create-russh-api'
// The setup file (test/renderer/helpers/setup.ts) injects window.russhApi automatically.
// To override behavior in a specific test, mock the method:
const api = createRuSSHApi({/* overrides */})
```

## Key Entry Points to Read First

- `src-tauri/src/lib.rs` — Rust runtime, command surface, events, tray, builder
- `src-tauri/compatibility.js` — how `window.russhApi` is implemented on Tauri
- `src/shared/api.ts` — canonical renderer-facing contract
- `src/shared/types.ts` — canonical shared types, event shapes, connection phases
- `src/renderer/src/App.tsx` — React root, theme bootstrapping, routing
- `src/renderer/src/components/workbench/workbench-shell.tsx` — renderer layout root
- `src/renderer/src/features/shared/api/russh-client.ts` — reference for how feature API clients wrap the bridge
