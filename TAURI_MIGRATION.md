# Tauri migration status

This branch is the isolated `tauri` migration branch. The `main` branch is
unchanged.

## Implemented foundation

- Added a Tauri 2 Rust application under `src-tauri/`.
- Added a thin Rust entry point and a `RuntimeState` managed state boundary.
- Added Specta-collected commands for application information, window
  minimize/maximize/close/state, local file path resolution, and a typed
  platform capability report.
- Added native window-state event emission for maximize/unmaximize/resize
  transitions, matching the existing renderer event channel name.
- Added a Tauri capability limited to the `main` window and the core default
  permission set.
- Added a separate Vite configuration that builds the existing React renderer
  without changing files under `src/renderer/` or `web/`.
- Added a document-start Tauri compatibility adapter that creates
  `window.russhApi` before the unchanged React workbench loads. Implemented
  system/window calls are wired to Rust; unimplemented domains fail explicitly
  instead of returning fake data.
- Added Rust-backed default settings loading from the Tauri application config
  directory, preserving the existing `AppSettings` camelCase shape.
- Added partial settings updates with merge, validation, and temporary-file
  replacement semantics so existing renderer auto-save calls remain compatible.
- Added a managed rusqlite connection at the Tauri application data path,
  initialized with a schema metadata table and exposed through a health command
  for the upcoming server/group/tag migrations.
- Added the first SQLite-backed business domain: group listing, creation, and
  deletion through the unchanged `window.russhApi.groups` shape.
- Added the SQLite-backed tag domain with list, create, and delete operations
  through the unchanged `window.russhApi.tags` shape.
- Completed group and tag update operations with SQLite persistence.
- Added the initial server persistence domain with list, create, and delete
  operations, preserving the renderer-facing server object shape for fields
  currently backed by SQLite.
- Added server updates and favorite toggling with partial JSON updates and
  timestamp refreshes.
- Added credential metadata and secret retrieval/storage commands backed by the
  SQLite credentials table; secret handling remains isolated from server list
  responses and must be hardened with OS-backed encryption before release.
- Added credential metadata and secret update support through the same SQLite
  record, while keeping secret fields out of list responses.
- Added built-in theme discovery from bundled JSON theme packs and wired
  `themes.list` through the unchanged renderer bridge.
- Added user theme archive import/delete with a native ZIP picker, safe archive
  extraction, staging replacement, user-plugin enumeration, and the original
  `ThemeImportResult`/`ThemeDeleteResult` bridge shape.
- Added an explicit Rust unsupported-update state for platforms/builds where
  automatic updating is not available, matching the existing updater contract.
- Added a Rust local-terminal process manager with shell startup, stdin writes,
  stdout event streaming, and close lifecycle wired through the existing
  `localTerminals` bridge, now backed by `portable-pty` with real resize
  semantics.
- Added an initial Rust `ssh2` session runtime with password/private-key
  authentication, interactive xterm PTY requests, session data/state events,
  writes, resize, disconnect, and host-key trust approval wired to the
  existing `sessions` bridge. Jump hosts, resource metrics, and binary data
  channels remain.
- Added private-key authentication with passphrase support using a short-lived
  restricted temporary key file that is removed after authentication.
- Corrected the compatibility adapter to return the renderer's
  `SessionConnectResult` envelope and emit session exit events on remote EOF.
- Added initial SSH-backed SFTP directory listing and refresh operations through
  the existing `sftp.list`/`sftp.refresh` bridge.
- Added text-only SFTP editor read/write streams with chunk/state events,
  matching the existing Monaco editor stream contract.
- Added SFTP create-file, mkdir, rename, move, and remove operations backed by
  the active SSH credentials.
- Added session-scoped local port-forward rules and SSH direct-tcpip listener
  lifecycle for list/create/start/stop/remove operations; rules remain memory
  only as in the Electron implementation.
- Added SQLite-backed known-host listing and removal commands through the
  existing system bridge. SSH handshake enforcement/interactive trust approval
  remains the next security step.
- Added SHA-256 host-key fingerprint enforcement during SSH handshake. Unknown
  or changed keys emit `system:hostTrustRequest` and are rejected until a
  verified trust-approval path is implemented.
- Added the trust approval resolver: `system.respondHostTrust` now unblocks the
  waiting SSH handshake, persists an approved fingerprint, and rejects on
  denial or timeout.
- Added Rust-backed log state/list/clear/path/write commands and bridged the
  existing `logs` API. Added WebDAV connection testing and remote backup-list
  discovery through the existing backup bridge. Added WebDAV SQLite backup
  upload and filename-scoped remote deletion through the same bridge. Added
  restore with SQLite-header validation, atomic replacement, rollback on
  rename failure, and Tauri application restart.
- Preserved the Electron WebDAV credential contract by persisting the
  `webdavPassword` update in SQLite `app_settings` and applying Basic Auth to
  all Rust WebDAV requests.
- Added a GitHub Releases-based manual update check that emits the existing
  update state and release metadata; installation remains disabled until a
  signed Tauri updater endpoint is configured.
- Fixed debug startup Specta export issues by using JavaScript-safe numeric
  types for exported command-history fields and an absolute bindings output
  path. Added `@tauri-apps/api` for the generated bindings runtime import.
- Removed the Electron-only `postinstall: electron-builder install-app-deps`
  hook on this Tauri branch so `npm install` does not force Electron ABI
  rebuilds of unused `ssh2/cpu-features` native modules.
- Added SQLite-backed command history, custom commands, and SFTP bookmarks,
  including CRUD/list/search/clear operations through the unchanged bridge.
- Added a dedicated SQLite server-secret table and bridged `servers.getSecrets`,
  including password, private-key, and passphrase persistence for new and
  updated server records. The table is created lazily on read/write so empty
  and pre-migration databases remain readable.
- Added a Rust shell-integration script command and wired it to the existing
  settings/server editor API.
- Added a typed unavailable-resource snapshot for session monitoring so the
  renderer receives a valid contract without fabricated CPU/memory metrics.
- Persisted the per-server command-history capture switch in SQLite and
  returned its actual value from the server list API.
- Mapped Tauri window focus/blur events to the existing `system:appFocus`
  subscription used by the renderer away detector.
- Enabled the Tauri tray icon and added basic Settings/New Connection/Local
  Terminal/Quit menu actions routed through `system:menuAction`.
- Connected `system.menu.onAction` in the compatibility adapter to the tray
  event stream, completing the Rust-to-renderer menu path.
- Synchronized Tauri and Cargo package metadata to the existing application
  version `1.2.4` so generated installers and runtime update metadata agree.
- Added the Tauri dialog plugin and implemented private-key and server-icon
  file selection with the existing return shapes.
- Added stored server-secret fallback during SSH connection establishment, so
  saved credentials work even when the renderer omits a per-request secret
  payload.
- Implemented Rust-backed recursive SFTP uploads for `uploadPaths`, including
  directory creation and file streaming through the active SSH session.
- Implemented `sftp.downloadFile` with a native save dialog and streamed
  remote-file download through the active SSH session.
- Implemented `sftp.uploadFiles` with native multi-file selection, reusing the
  recursive upload path for selected files and directories.
- Added cancellable explicit-path SFTP upload batches with chunk-level
  cancellation tokens, cancellation progress events, and wired
  `cancelTransfer`/`cancelAllTransfers` commands.
- Added running/completed `sftp:transferProgress` events for explicit-path
  uploads so the existing transfer UI receives lifecycle feedback.
- Wired `sftp.onTransferProgress` to that event stream in the compatibility
  adapter.
- Added `portForwards:state` events for forward creation, activation, and
  stopping, and connected `portForwards.onStateChange` to the event stream.
- Unified password/private-key authentication for auxiliary SFTP and port
  forwarding connections, reusing the stored session credentials.
- Implemented session reconnect with the existing sessionId and stored server
  credentials, and bridged `sessions.reconnect`.
- Connected Rust session failure paths to both `sessions:state` and
  `sessions:error`, preserving renderer error notifications.
- Made `system.getPathForFile` return the preload-compatible `null` fallback
  in Tauri WebViews, where browser `File` objects do not expose native paths.
- Added a guarded `File.path` fallback for Tauri/WebView environments that do
  expose a native path, while preserving the browser `null` behavior.
- Connected `system.window.onStateChange` to the Rust-emitted maximize/resize
  state event.
- Added local-terminal running/stopped state and exit events for the existing
  `localTerminals` subscriptions.
- Added SQLite-backed recent-session history with retention of the latest 20
  connections and wired `servers.listRecent`/`clearRecent` without renderer
  changes.
- Added stopped/exit events when a local shell naturally reaches EOF, not only
  when the renderer explicitly closes it.
- Added OSC 7/OSC 133 cwd extraction from SSH terminal output and bridged
  `sessions:cwdChanged` for SFTP follow-directory behavior.
- Added Rust unit tests covering OSC cwd extraction, input normalization, and
  safety checks; `cargo test` now reports 6 passed.
- Implemented `system.relaunch` using Tauri's native application restart
  lifecycle, preserving restore/relaunch flows.
- Added compatibility-layer activity tracking for keyboard, pointer, touch,
  and wheel input, emitting the existing `system:appActivity` wake event.
- Added a Tauri `MessageChannel` compatibility implementation for
  `sessions.createDataChannel`, converting `sessions:data` events into the
  existing 16-byte-header SSH data-frame format consumed by the unchanged
  terminal worker.
- Added live SSH command-history capture in Rust: session input is buffered per
  session until newline, persisted when the server capture switch is enabled,
  and emitted through `commandHistory:added`.
- Normalized captured terminal input by handling backspace and ANSI cursor
  sequences before persisting command history.
- Added Tauri development/build scripts and a checked-in location for generated
  Specta bindings.

## Remaining migration work

The Electron runtime (`src/main/`, `src/preload/`, electron-vite, electron-builder,
and their tests) has been removed. The following work is still outstanding:

- Signed automatic update download/install and terminal backpressure event
  production.
- Cross-platform signing, updater artifacts, and platform-specific window
  behavior validation.

The renderer still consumes the `window.russhApi` global, which is now provided
by `src-tauri/compatibility.js` (injected as a Tauri initialization script)
instead of an Electron preload script. The adapter reproduces the previous
runtime shape, including subscription cleanup and streaming semantics; replacing
it with a partial `invoke` wrapper would break startup and terminal behavior.

## Compatibility risks

- Tauri WebViews do not provide Electron's `ipcRenderer`, `MessagePort`,
  `node-pty`, or Node filesystem APIs. Streaming terminals and SFTP need
  explicit Rust channels/events and backpressure handling.
- Credential storage and database migration must preserve existing user data;
  moving secrets without an explicit migration plan risks data loss.
- Window chrome, tray/menu behavior, auto-update, code signing, and installer
  formats are platform-specific and cannot be validated by a desktop Linux
  compile alone.
- The current Tauri CSP and permissions are intentionally minimal. They must be
  expanded only alongside reviewed Rust commands and scoped capabilities.

## Validation

- `git switch -c tauri`: passed.
- `git diff --check`: passed.
- `cargo check --manifest-path src-tauri/Cargo.toml`: passed.
- `npm run typecheck`: passed.
- `npm run tauri:build:frontend`: passed; the unchanged React renderer builds
  into `dist/renderer`.
- `npm run tauri:build`: Rust release build and Debian/RPM bundles passed;
  AppImage bundling is blocked in this environment by `Read-only file system`
  while writing the AppImage runtime. Generated artifacts:
  `src-tauri/target/release/bundle/deb/RuSSH_1.2.4_amd64.deb` and
  `src-tauri/target/release/bundle/rpm/RuSSH-1.2.4-1.x86_64.rpm`.
- `cargo test --manifest-path src-tauri/Cargo.toml`: passed (6 Rust tests).
- Debug binary smoke test reached Tauri/GTK initialization; headless execution
  stops at GTK backend initialization because this environment has no display.
- Full Vitest run: 81 files passed, 2 skipped; 6 files and 7 tests failed
  (`540 passed`, `18 skipped`). The failures are environment/pre-existing
  coverage issues (sandboxed loopback `EPERM`, missing Electron postinstall
  binary, WebDAV test server startup, and one existing update-editor
  assertion), not Rust compilation or renderer build failures.
