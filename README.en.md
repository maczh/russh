<p align="right">
  <a href="./README.md">简体中文</a> | <strong>English</strong>
</p>

<div align="center">
  <img src="resources/icon.png" width="120" alt="RuSSH Logo" />
  <h1>RuSSH</h1>
  <p>🚀 <strong>A modern, powerful, and beautiful cross-platform desktop SSH / SFTP client</strong> 🚀</p>
  <p>Built with Tauri 2 (Rust) · React 19 · TypeScript 5 · Tailwind CSS 4</p>

  <p>
    <a href="https://github.com/lantongxue/russh/releases">
      <img src="https://img.shields.io/badge/version-1.2.4-blue?style=flat-square&logo=gittag" alt="version" />
    </a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey?style=flat-square" alt="platform" />
    <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="license" />
    <img src="https://img.shields.io/badge/PRs-welcome-brightgreen?style=flat-square" alt="PRs Welcome" />
  </p>
</div>

---

## 📖 Introduction

**RuSSH** is a cross-platform SSH and SFTP client designed for developers and system administrators. Moving away from the cluttered and cumbersome UI of traditional terminal tools, we aim to integrate **SSH sessions, local terminals, SFTP file transfers, port forwarding, command palettes, credential vaults, away reminder**, and a **personalized theme system** all within a single window. No more frequent context-switching between tools — RuSSH delivers a fast, smooth, and immersive operations experience.

<div align="center">
  <img src="official-website/public/russh-shell-en.png" width="100%" alt="RuSSH Dark Preview" style="border-radius: 8px; box-shadow: 0 4px 20px rgba(0,0,0,0.15);" />
</div>

---

## ✨ Key Features

RuSSH is more than just a terminal emulator; it is a complete, closed-loop operations workbench:

- 🏠 **Unified Workbench** — Manage SSH remote sessions and local terminals under a single tabbed interface. Utilizing a **keep-mounted strategy**, terminal instances (xterm.js) remain resident in memory when switching tabs, preventing disruption to active commands and output context.
- 📁 **Deeply Integrated SFTP** — Provides a visual remote file manager supporting **two-way drag-and-drop upload/download** with local file explorers, batch actions, and permission management. **Integrated with Monaco Editor**, double-clicking remote text files allows inline editing with instant background sync on save.
- ⚡ **Command Palette & Custom Commands** — Summon a global command palette with a keyboard shortcut. Besides tracking command history, you can save frequently-used operations commands for one-click execution to boost efficiency.
- 🔒 **Credential Vault** — Safely hosts passwords and private keys to isolate sensitive account data, with support for Jump Server (single-hop proxy) traversal.
- 📊 **Real-time Host Resource Monitoring** — Ready to use out of the box with no remote agent installation required. Displays real-time charts in the sidebar or bottom panel showing Linux server CPU, memory, network throughput, and disk I/O metrics.
- 🌐 **Flexible Port Forwarding** — Supports Local Port Forwarding and Remote Port Forwarding, with automatic rule restoration after reconnection when network fluctuations occur.
- 🎨 **VSCode-style Themes** — Built-in collection of beautiful, high-contrast themes, supporting standard VSCode JSON theme imports and automatic syncing with system Dark/Light modes.
- ☁️ **WebDAV Cloud Sync** — Back up and restore host configurations, credentials, and preferences via WebDAV. Automatically performs a hot-relaunch upon successful configuration restoration.
- ⏰ **Away Reminder** — Detects user inactivity, displaying a protective overlay and locking the terminal windows to safeguard data privacy.

---

## 🛠️ Tech Stack

RuSSH is built on top of a mature and modern frontend ecosystem for high performance:

- **Shell Environment**: Tauri 2 (Rust)
- **Frontend Framework**: React 19 (Hooks) + TypeScript 5
- **Build System**: Vite 7 + Tauri CLI
- **CSS Styling**: Tailwind CSS 4
- **Core Dependencies**:
  - [xterm.js](https://github.com/xtermjs/xterm.js) - Terminal rendering engine supporting WebGL acceleration and Web fonts
  - [Monaco Editor](https://github.com/microsoft/monaco-editor) - Inline text editing for remote files
  - [ssh2](https://github.com/alexcrichton/ssh2-rs) - Rust SSH2 protocol implementation
  - [portable-pty](https://github.com/wez/wezterm/tree/main/pty) - Local pseudo-terminal bridge
  - [rusqlite](https://github.com/rusqlite/rusqlite) - Embedded SQLite driver (bundled)
  - [tauri-plugin-dialog](https://github.com/tauri-apps/plugins-workspace) - Native file open/save dialogs

---

## 🚀 Installation & Local Development

### Prerequisites

- **Node.js**: `22.x` or higher
- **npm**: `10.x` or higher
- **Rust**: stable toolchain (with `cargo`)
- **System libraries**: GTK / WebKitGTK development packages on Linux (see the Tauri 2 prerequisites)
- **Linux bundling**: `libayatana-appindicator3-dev` is required when bundling with the tray icon feature, otherwise `tauri build` aborts because pkg-config cannot locate appindicator

### Development Steps

1. **Clone the repository**:

   ```bash
   git clone https://github.com/lantongxue/russh.git
   cd russh
   ```

2. **Install dependencies**:

   ```bash
   npm install
   ```

3. **Start Tauri in development mode** (Vite frontend HMR + Rust backend):
   ```bash
   npm run dev
   ```

### Packaging & Distribution

This project uses the Tauri CLI to package applications for various platforms:

```bash
# Build Windows installer (NSIS)
npm run dist:win

# Build macOS bundles (APP & DMG)
npm run dist:mac

# Build Linux packages (AppImage, DEB & RPM)
npm run dist:linux
```

---

## 📂 Project Directory Structure

```text
russh/
├── src/
│   ├── renderer/          # React frontend - workbench interaction layer, state management, and feature API clients
│   └── shared/            # Shared codebase - validation schemas (Zod), global types, and IPC contracts
├── src-tauri/             # Tauri 2 / Rust backend - database, SSH/SFTP, local terminals, themes, updates
│   ├── src/               # Rust sources (commands, runtime state)
│   ├── capabilities/      # Tauri permission capabilities
│   ├── compatibility.js   # Frontend adapter that injects window.russhApi
│   └── tauri.conf.json    # Window, CSP, and bundling configuration
├── themes/                # Built-in VSCode-style JSON themes
├── docs/                  # Supplementary design and theme development guidelines
├── scripts/               # Tauri development launcher
└── build/                 # Bundling icon assets
```

---

## 💻 Script Reference

| Command               | Description                                                       |
| :-------------------- | :---------------------------------------------------------------- |
| `npm run dev`         | Starts Tauri development (Vite frontend + Rust backend)           |
| `npm run build`       | Runs TypeScript type checking and builds the frontend assets      |
| `npm run test`        | Runs all Vitest unit and integration tests                        |
| `npm run typecheck`   | Runs TS type checks for the shared/config layer and the renderer  |
| `npm run format`      | Formats all code with Prettier to maintain a consistent style     |
| `npm run lint`        | Runs ESLint to check for code style issues and rules violation    |
| `npm run tauri:build` | Builds the Rust backend and produces platform installers          |

---

## 🤝 Contributing

If you encounter any bugs or have ideas for improvements, feel free to open an issue or submit a pull request!

Before submitting a PR, please make sure all local checks pass successfully:

```bash
# Run type checks and all unit/integration tests
npm run typecheck && npm run test
```

- **Commit Message Guidelines**: Please follow the [Conventional Commits](https://www.conventionalcommits.org/) specification (e.g., `feat: add xterm font size configuration`).
- **Code Formatting**: Run `npm run format` to format your code before committing.

---

<div align="center">
  <p>Made with ❤️ by the RuSSH Team.</p>
  <p>Released under the <a href="LICENSE.en">License</a>.</p>
</div>
