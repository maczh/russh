# Tauri runtime

This directory is the native-runtime migration boundary for the `tauri` branch.
Rust commands use Tauri Specta and are the source of truth for the generated
TypeScript contract.

The existing React workbench is intentionally not copied or rewritten. The
Electron preload bridge remains the compatibility contract to be implemented by
the Tauri adapter before the Electron runtime can be removed.
