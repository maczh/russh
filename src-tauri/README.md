# Tauri runtime

This directory is the native runtime for RuSSH.

- `src/lib.rs` — `RuntimeState`, Tauri commands, event emitters, tray menu, and the application builder.
- `src/main.rs` — thin entry that calls `russh_tauri_lib::run()`.
- `compatibility.js` — injected at document start by `CompatibilityPlugin`; implements `window.russhApi` on top of Tauri `invoke` and events.
- `capabilities/default.json` — permissions scoped to the `main` window.
- `gen/schemas/` — generated capability schemas.
- `tauri.conf.json` — window, CSP, dev URL, bundling, and icons.

Rust commands use Tauri Specta and are the source of truth for the generated
TypeScript contract in `src/shared/tauri-bindings.ts`. The React workbench is not
rewritten: it consumes `window.russhApi`, whose shape is defined in
`src/shared/api.ts`.
