use serde::Serialize;
use serde::{Deserialize, Serialize as SerdeSerialize};
use std::sync::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use rusqlite::{Connection, OptionalExtension};
use specta::Type;
#[cfg(debug_assertions)]
use specta_typescript::Typescript;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri::menu::MenuBuilder;
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri_specta::{collect_commands, Builder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use zip::ZipArchive;

pub struct RuntimeState {
    database: Mutex<Connection>,
    local_terminals: Mutex<HashMap<String, LocalTerminalProcess>>,
    sessions: Mutex<HashMap<String, Sender<SessionCommand>>>,
    connection_info: Mutex<HashMap<String, SessionConnectionInfo>>,
    sftp_streams: Mutex<HashMap<String, SftpStream>>,
    port_forward_rules: Mutex<HashMap<String, PortForwardRecord>>,
    port_forward_stops: Mutex<HashMap<String, Sender<()>>>,
    pending_trust: Arc<Mutex<HashMap<String, Sender<bool>>>>,
    log_path: Mutex<String>,
    logs: Mutex<Vec<LogRecord>>,
    transfer_cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    command_buffers: Mutex<HashMap<String, String>>,
    session_output_buffers: Arc<Mutex<HashMap<String, String>>>,
}

struct LocalTerminalProcess {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

enum SessionCommand {
    Write(Vec<u8>),
    Resize(u16, u16),
    Close,
}

#[derive(Clone)]
struct SessionConnectionInfo {
    server_id: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    private_key: Option<String>,
    passphrase: Option<String>,
    jump: Option<Box<SessionConnectionInfo>>,
}

enum SftpStream {
    Read { info: SessionConnectionInfo, session_id: String, remote_path: String },
    Write { info: SessionConnectionInfo, session_id: String, remote_path: String, buffer: String },
}

#[derive(Clone)]
struct PortForwardRecord {
    id: String,
    session_id: String,
    bind_host: String,
    bind_port: u16,
    target_host: String,
    target_port: u16,
    kind: String,
    enabled: bool,
}

impl RuntimeState {
    fn open(app: &AppHandle) -> Result<Self, String> {
        let data_dir = app.path().app_data_dir().map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
        let database = Connection::open(data_dir.join("russh.db")).map_err(|error| error.to_string())?;
        database
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\n                 INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('schema_version', '1');\n                 CREATE TABLE IF NOT EXISTS groups (id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT, color TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);\n                 CREATE TABLE IF NOT EXISTS tags (id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);\n                 CREATE TABLE IF NOT EXISTS servers (id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, username TEXT NOT NULL, auth_type TEXT NOT NULL, favorite INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);\n                 CREATE TABLE IF NOT EXISTS credentials (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, username TEXT, password TEXT, private_key TEXT, passphrase TEXT, note TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);\n                 CREATE TABLE IF NOT EXISTS known_hosts (host TEXT NOT NULL, port INTEGER NOT NULL, algorithm TEXT NOT NULL, fingerprint TEXT NOT NULL, verified_at TEXT NOT NULL, PRIMARY KEY (host, port));\n                 CREATE TABLE IF NOT EXISTS command_history (id TEXT PRIMARY KEY, scope_kind TEXT NOT NULL, server_id TEXT, command TEXT NOT NULL, executed_at TEXT NOT NULL, cwd TEXT, exit_code INTEGER, duration_ms INTEGER);\n                 CREATE TABLE IF NOT EXISTS custom_commands (id TEXT PRIMARY KEY, name TEXT NOT NULL, command TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);\n                 CREATE TABLE IF NOT EXISTS sftp_bookmarks (id TEXT PRIMARY KEY, server_id TEXT NOT NULL, path TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE(server_id, path));",
            )
            .map_err(|error| error.to_string())?;
        database.execute("ALTER TABLE servers ADD COLUMN jump_server_id TEXT", []).or_else(|error| {
            if error.to_string().contains("duplicate column name") { Ok(0) } else { Err(error) }
        }).map_err(|error: rusqlite::Error| error.to_string())?;
        database.execute("ALTER TABLE servers ADD COLUMN group_id TEXT", []).or_else(|error| {
            if error.to_string().contains("duplicate column name") { Ok(0) } else { Err(error) }
        }).map_err(|error: rusqlite::Error| error.to_string())?;
        database.execute("CREATE TABLE IF NOT EXISTS server_preferences (server_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 1)", []).map_err(|error| error.to_string())?;
        database.execute("CREATE TABLE IF NOT EXISTS recent_sessions (id TEXT PRIMARY KEY, server_id TEXT NOT NULL, connected_at TEXT NOT NULL)", []).map_err(|error| error.to_string())?;
        Ok(Self { database: Mutex::new(database), local_terminals: Mutex::new(HashMap::new()), sessions: Mutex::new(HashMap::new()), connection_info: Mutex::new(HashMap::new()), sftp_streams: Mutex::new(HashMap::new()), port_forward_rules: Mutex::new(HashMap::new()), port_forward_stops: Mutex::new(HashMap::new()), pending_trust: Arc::new(Mutex::new(HashMap::new())), log_path: Mutex::new(data_dir.join("russh.log").to_string_lossy().into_owned()), logs: Mutex::new(Vec::new()), transfer_cancellations: Mutex::new(HashMap::new()), command_buffers: Mutex::new(HashMap::new()), session_output_buffers: Arc::new(Mutex::new(HashMap::new())) })
    }
}

struct CompatibilityPlugin;

impl<R: Runtime> tauri::plugin::Plugin<R> for CompatibilityPlugin {
    fn name(&self) -> &'static str {
        "russh-compatibility"
    }

    fn initialization_script(&self) -> Option<String> {
        Some(include_str!("../compatibility.js").to_owned())
    }
}

#[derive(Debug, Serialize, Type)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub release_channel: String,
}

#[derive(Debug, Serialize, Type)]
pub struct SystemCapabilities {
    pub platform: String,
    pub supports_tray: bool,
    pub supports_native_menu: bool,
    pub supports_local_terminal: bool,
    pub supports_ssh: bool,
    pub supports_sftp: bool,
}

#[derive(Debug, Serialize, Type, Clone, Copy)]
pub struct WindowState {
    pub is_maximized: bool,
}

#[derive(Debug, Clone, Deserialize, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub language: String,
    pub log_file_path: Option<String>,
    pub theme: String,
    pub terminal_font_size: u32,
    pub ui_font_id: String,
    pub terminal_font_id: String,
    pub editor_font_id: Option<String>,
    pub auto_update_check_enabled: bool,
    pub experimental_terminal_webgl: bool,
    pub cursor_style: String,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub local_terminal_shell: String,
    pub window_title_bar_style: String,
    pub webdav_backup_enabled: bool,
    pub webdav_url: Option<String>,
    pub webdav_username: Option<String>,
    pub webdav_backup_interval_minutes: u32,
    pub webdav_backup_path: Option<String>,
    pub resource_monitor_interval_ms: u32,
    pub sftp_upload_concurrency: u32,
    pub sftp_download_concurrency: u32,
    pub command_history_enabled: bool,
    pub proxy_mode: String,
    pub proxy_type: String,
    pub proxy_host: String,
    pub proxy_port: u16,
}

#[derive(Debug, Clone, Deserialize, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupRecord {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub color: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagRecord {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub username: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSecretRecord {
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostRecord {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub verified_at: String,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStateRecord {
    pub phase: String,
    pub supported: bool,
    pub current_version: String,
    pub auto_check_enabled: bool,
    pub available_update: Option<UpdateVersionRecord>,
    pub download_progress_percent: Option<f64>,
    pub error_message: Option<String>,
    pub unsupported_reason: Option<String>,
    pub requires_manual_install: bool,
    pub releases_url: Option<String>,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVersionRecord {
    pub version: String,
    pub release_date: Option<String>,
    pub release_name: Option<String>,
    pub release_notes: Option<String>,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub id: String,
    pub level: Option<String>,
    pub message: String,
    pub raw: String,
    pub source: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryRecord { pub id: String, pub scope_kind: String, pub server_id: Option<String>, pub command: String, pub executed_at: String, pub cwd: Option<String>, pub exit_code: Option<i32>, pub duration_ms: Option<i32> }

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomCommandRecord { pub id: String, pub name: String, pub command: String, pub created_at: String, pub updated_at: String }

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpBookmarkRecord { pub id: String, pub server_id: String, pub path: String, pub created_at: String }

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerIconRecord { pub mime_type: String, pub data: Vec<u8> }

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummaryRecord {
    pub session_id: String,
    pub server_id: String,
    pub server_name: String,
    pub host: String,
    pub port: u16,
    pub status: String,
    pub connected_at: String,
    pub current_path: String,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalTerminalSummary {
    pub terminal_id: String,
    pub title: String,
    pub shell: String,
    pub cwd: String,
    pub started_at: String,
    pub status: String,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentServerRecord {
    pub id: String,
    pub server_id: String,
    pub connected_at: String,
    pub server_name: String,
    pub host: String,
}

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebDavTestRecord { pub ok: bool, pub message: String }

#[derive(Debug, Clone, SerdeSerialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WebDavBackupRecord { pub file_name: String, pub modified_at: String }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            log_file_path: None,
            theme: "system".into(),
            terminal_font_size: 14,
            ui_font_id: "inter".into(),
            terminal_font_id: "jetbrains-mono".into(),
            editor_font_id: Some("source-sans-3".into()),
            auto_update_check_enabled: true,
            experimental_terminal_webgl: false,
            cursor_style: "block".into(),
            cursor_blink: true,
            copy_on_select: true,
            local_terminal_shell: "zsh".into(),
            window_title_bar_style: "native".into(),
            webdav_backup_enabled: false,
            webdav_url: None,
            webdav_username: None,
            webdav_backup_interval_minutes: 60,
            webdav_backup_path: Some("/russh-backup/".into()),
            resource_monitor_interval_ms: 2000,
            sftp_upload_concurrency: 3,
            sftp_download_concurrency: 3,
            command_history_enabled: true,
            proxy_mode: "none".into(),
            proxy_type: "socks5".into(),
            proxy_host: "127.0.0.1".into(),
            proxy_port: 1080,
        }
    }
}

#[tauri::command]
#[specta::specta]
fn get_app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        name: "RuSSH".into(),
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS.into(),
        release_channel: "latest".into(),
    }
}

#[tauri::command]
#[specta::specta]
fn get_system_capabilities() -> SystemCapabilities {
    SystemCapabilities {
        platform: std::env::consts::OS.into(),
        supports_tray: true,
        supports_native_menu: true,
        supports_local_terminal: cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux")),
        supports_ssh: true,
        supports_sftp: true,
    }
}

#[tauri::command]
#[specta::specta]
fn list_themes(app: AppHandle) -> Result<String, String> {
    let candidates = [
        ("russh-default-themes", "RuSSH Default Themes", "light-plus", "Light+", "light", "vs"),
        ("russh-default-themes", "RuSSH Default Themes", "dark-plus", "Dark+", "dark", "vs-dark"),
        ("russh-default-themes", "RuSSH Default Themes", "pixel-crt", "Pixel CRT", "dark", "vs-dark"),
        ("russh-high-contrast", "RuSSH High Contrast", "high-contrast-light", "High Contrast Light", "light", "hc-light"),
        ("russh-high-contrast", "RuSSH High Contrast", "high-contrast-dark", "High Contrast Dark", "dark", "hc-black"),
    ];
    let mut themes = Vec::new();
    for (plugin_id, plugin_name, file, label, appearance, ui_theme) in candidates {
        let theme: serde_json::Value = serde_json::from_str(builtin_theme_document(file)).map_err(|error| error.to_string())?;
        themes.push(serde_json::json!({"id": format!("russh.{}", file), "label": label, "description": label, "pluginId": plugin_id, "pluginDisplayName": plugin_name, "version": "1.0.0", "appearance": appearance, "uiTheme": ui_theme, "source": "builtin", "colors": theme.get("colors").cloned().unwrap_or_default(), "terminal": theme.get("terminal").cloned().unwrap_or_default(), "terminalDefaults": theme.get("terminalDefaults").cloned()}));
    }
    if let Ok(user_root) = theme_user_root(&app) {
        if let Ok(entries) = std::fs::read_dir(user_root) {
            for entry in entries.flatten() {
                let manifest_path = entry.path().join("package.json");
                let Ok(manifest) = std::fs::read_to_string(manifest_path).ok().and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok()).ok_or(()) else { continue };
                let plugin_id = manifest.get("name").and_then(|value| value.as_str()).unwrap_or_default();
                let plugin_name = manifest.get("displayName").and_then(|value| value.as_str()).unwrap_or(plugin_id);
                let version = manifest.get("version").and_then(|value| value.as_str()).unwrap_or("1.0.0");
                let Some(contributions) = manifest.get("contributes").and_then(|value| value.get("themes")).and_then(|value| value.as_array()) else { continue };
                for contribution in contributions {
                    let Some(theme_id) = contribution.get("id").and_then(|value| value.as_str()) else { continue };
                    let Some(theme_path) = contribution.get("path").and_then(|value| value.as_str()) else { continue };
                    let path = entry.path().join(theme_path);
                    let Ok(document) = std::fs::read_to_string(path).ok().and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok()).ok_or(()) else { continue };
                    let ui_theme = contribution.get("uiTheme").and_then(|value| value.as_str()).unwrap_or("vs-dark");
                    let appearance = if ui_theme == "vs" || ui_theme == "hc-light" { "light" } else { "dark" };
                    themes.push(serde_json::json!({"id": theme_id, "label": contribution.get("label").and_then(|value| value.as_str()).unwrap_or(theme_id), "description": contribution.get("description"), "pluginId": plugin_id, "pluginDisplayName": plugin_name, "version": version, "appearance": appearance, "uiTheme": ui_theme, "source": "user", "colors": document.get("colors").cloned().unwrap_or_default(), "terminal": document.get("terminal").cloned().unwrap_or_default(), "terminalDefaults": document.get("terminalDefaults").cloned()}));
                }
            }
        }
    }
    serde_json::to_string(&themes).map_err(|error| error.to_string())
}

// Built-ins must work when a standalone binary is started outside the source tree.
fn builtin_theme_document(name: &str) -> &'static str {
    match name {
        "light-plus" => include_str!("../../themes/builtin/russh-default-themes/themes/light-plus.json"),
        "dark-plus" => include_str!("../../themes/builtin/russh-default-themes/themes/dark-plus.json"),
        "pixel-crt" => include_str!("../../themes/builtin/russh-default-themes/themes/pixel-crt.json"),
        "high-contrast-light" => include_str!("../../themes/builtin/russh-high-contrast/themes/high-contrast-light.json"),
        "high-contrast-dark" => include_str!("../../themes/builtin/russh-high-contrast/themes/high-contrast-dark.json"),
        _ => unreachable!("unknown built-in theme"),
    }
}

#[cfg(test)]
mod embedded_theme_tests {
    #[test]
    fn builtin_themes_include_terminal_colors_without_filesystem_access() {
        for name in ["light-plus", "dark-plus", "pixel-crt", "high-contrast-light", "high-contrast-dark"] {
            let document: serde_json::Value = serde_json::from_str(super::builtin_theme_document(name)).unwrap();
            assert!(document["colors"].is_object(), "missing colors: {name}");
            assert!(document["terminal"].is_object(), "missing terminal colors: {name}");
        }
    }
}

fn theme_user_root(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let root = app.path().app_data_dir().map_err(|error| error.to_string())?.join("themes");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

fn safe_theme_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && value.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

#[tauri::command]
#[specta::specta]
fn import_theme_archive(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = mpsc::channel();
    app.dialog().file().add_filter("Theme archive", &["zip"]).pick_file(move |path| { let _ = sender.send(path.map(|value| value.to_string())); });
    let Some(path) = receiver.recv_timeout(std::time::Duration::from_secs(300)).map_err(|error| error.to_string())? else { return Ok(None); };
    let archive_file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;
    let root = theme_user_root(&app)?;
    let staging = root.join(format!(".stage-{}", utc_timestamp().replace(':', "-")));
    std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(enclosed) = entry.enclosed_name().map(|value| value.to_path_buf()) else { return Err("theme archive contains an unsafe path".into()); };
        let destination = staging.join(enclosed);
        if entry.is_dir() { std::fs::create_dir_all(&destination).map_err(|error| error.to_string())?; } else {
            if let Some(parent) = destination.parent() { std::fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
            let mut output = std::fs::File::create(&destination).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
        }
    }
    let manifest_path = [staging.join("package.json"), staging.join("manifest.json")].into_iter().find(|path| path.exists()).ok_or("theme archive manifest is missing")?;
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let plugin_id = manifest.get("name").and_then(|value| value.as_str()).ok_or("theme archive name is missing")?;
    if !safe_theme_component(plugin_id) { return Err("theme archive name is unsafe".into()); }
    let display_name = manifest.get("displayName").and_then(|value| value.as_str()).unwrap_or(plugin_id).to_owned();
    let destination = root.join(plugin_id);
    if destination.exists() { std::fs::remove_dir_all(&destination).map_err(|error| error.to_string())?; }
    std::fs::rename(&staging, &destination).map_err(|error| error.to_string())?;
    Ok(Some(serde_json::json!({"pluginId": plugin_id, "pluginDisplayName": display_name, "themes": []}).to_string()))
}

#[tauri::command]
#[specta::specta]
fn delete_theme_plugin(app: AppHandle, plugin_id: String) -> Result<String, String> {
    if !safe_theme_component(&plugin_id) { return Err("theme plugin id is unsafe".into()); }
    let path = theme_user_root(&app)?.join(&plugin_id);
    if !path.exists() { return Err("theme plugin was not found".into()); }
    std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    serde_json::to_string(&serde_json::json!({"pluginId": plugin_id, "pluginDisplayName": plugin_id, "deletedThemeIds": [], "nextThemeSelection": null})).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn get_update_state(app: AppHandle) -> UpdateStateRecord {
    let state = UpdateStateRecord { phase: "unsupported".into(), supported: false, current_version: app.package_info().version.to_string(), auto_check_enabled: false, available_update: None, download_progress_percent: None, error_message: None, unsupported_reason: Some("platform_not_supported".into()), requires_manual_install: false, releases_url: None };
    let _ = app.emit("updates:state", state.clone());
    state
}

#[tauri::command]
#[specta::specta]
fn check_for_updates(app: AppHandle) -> Result<UpdateStateRecord, String> {
    let current = app.package_info().version.to_string();
    let response = reqwest::blocking::Client::new().get("https://api.github.com/repos/lantongxue/russh/releases/latest").header("User-Agent", "RuSSH-Tauri").send().map_err(|error| error.to_string())?;
    if !response.status().is_success() { return Err(format!("update check failed with HTTP {}", response.status())); }
    let release: serde_json::Value = response.json().map_err(|error| error.to_string())?;
    let version = release.get("tag_name").and_then(|value| value.as_str()).unwrap_or_default().trim_start_matches('v').to_owned();
    let newer = !version.is_empty() && version != current;
    let state = UpdateStateRecord { phase: if newer { "available" } else { "idle" }.into(), supported: false, current_version: current, auto_check_enabled: true, available_update: if newer { Some(UpdateVersionRecord { version, release_date: release.get("published_at").and_then(|value| value.as_str()).map(str::to_owned), release_name: release.get("name").and_then(|value| value.as_str()).map(str::to_owned), release_notes: release.get("body").and_then(|value| value.as_str()).map(str::to_owned) }) } else { None }, download_progress_percent: None, error_message: None, unsupported_reason: Some("tauri_signing_not_configured".into()), requires_manual_install: newer, releases_url: release.get("html_url").and_then(|value| value.as_str()).map(str::to_owned) };
    let _ = app.emit("updates:state", state.clone());
    Ok(state)
}

#[tauri::command]
#[specta::specta]
fn get_log_state(state: tauri::State<'_, RuntimeState>) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({"logFilePath": state.log_path.lock().map_err(|error| error.to_string())?.clone()})).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_logs(state: tauri::State<'_, RuntimeState>) -> Result<Vec<LogRecord>, String> {
    Ok(state.logs.lock().map_err(|error| error.to_string())?.clone())
}

#[tauri::command]
#[specta::specta]
fn clear_logs(state: tauri::State<'_, RuntimeState>) -> Result<(), String> {
    state.logs.lock().map_err(|error| error.to_string())?.clear();
    let path = state.log_path.lock().map_err(|error| error.to_string())?.clone();
    let _ = std::fs::write(path, "");
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn update_log_path(state: tauri::State<'_, RuntimeState>, path: String) -> Result<String, String> {
    *state.log_path.lock().map_err(|error| error.to_string())? = path.clone();
    serde_json::to_string(&serde_json::json!({"logFilePath": path})).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn write_log(state: tauri::State<'_, RuntimeState>, event: String) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(&event).map_err(|error| error.to_string())?;
    let record = LogRecord { id: format!("log-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()), level: value.get("level").and_then(|value| value.as_str()).map(str::to_owned), message: value.get("message").and_then(|value| value.as_str()).unwrap_or_default().into(), raw: event, source: value.get("source").and_then(|value| value.as_str()).map(str::to_owned), timestamp: value.get("timestamp").and_then(|value| value.as_str()).map(str::to_owned) };
    let path = state.log_path.lock().map_err(|error| error.to_string())?.clone();
    if let Some(parent) = std::path::Path::new(&path).parent() { let _ = std::fs::create_dir_all(parent); }
    let line = format!("{}\n", record.raw);
    use std::fs::OpenOptions;
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) { let _ = file.write_all(line.as_bytes()); }
    state.logs.lock().map_err(|error| error.to_string())?.push(record);
    Ok(())
}

fn history_scope(value: &serde_json::Value) -> (String, Option<String>) {
    let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("local").to_owned();
    let server = value.get("serverId").and_then(|v| v.as_str()).map(str::to_owned);
    (kind, server)
}

#[tauri::command]
#[specta::specta]
fn list_command_history(state: tauri::State<'_, RuntimeState>, input: String) -> Result<Vec<CommandHistoryRecord>, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    let (kind, server) = history_scope(value.get("scope").unwrap_or(&value));
    let limit = value.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 500);
    let db = state.database.lock().map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT id, scope_kind, server_id, command, executed_at, cwd, exit_code, duration_ms FROM command_history WHERE scope_kind = ?1 AND (server_id IS ?2 OR server_id = ?2) ORDER BY executed_at DESC LIMIT ?3").map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params![kind, server, limit], |row| Ok(CommandHistoryRecord { id: row.get(0)?, scope_kind: row.get(1)?, server_id: row.get(2)?, command: row.get(3)?, executed_at: row.get(4)?, cwd: row.get(5)?, exit_code: row.get(6)?, duration_ms: row.get(7)? })).map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn search_command_history(state: tauri::State<'_, RuntimeState>, input: String) -> Result<Vec<CommandHistoryRecord>, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    let (kind, server) = history_scope(value.get("scope").unwrap_or(&value));
    let query = value.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = value.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 500);
    let db = state.database.lock().map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT id, scope_kind, server_id, command, executed_at, cwd, exit_code, duration_ms FROM command_history WHERE scope_kind = ?1 AND (server_id IS ?2 OR server_id = ?2) AND command LIKE ?3 ORDER BY executed_at DESC LIMIT ?4").map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params![kind, server, format!("%{query}%"), limit], |row| Ok(CommandHistoryRecord { id: row.get(0)?, scope_kind: row.get(1)?, server_id: row.get(2)?, command: row.get(3)?, executed_at: row.get(4)?, cwd: row.get(5)?, exit_code: row.get(6)?, duration_ms: row.get(7)? })).map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn clear_command_history(state: tauri::State<'_, RuntimeState>, scope: String) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(&scope).map_err(|e| e.to_string())?;
    let (kind, server) = history_scope(&value);
    state.database.lock().map_err(|e| e.to_string())?.execute("DELETE FROM command_history WHERE scope_kind = ?1 AND (server_id IS ?2 OR server_id = ?2)", rusqlite::params![kind, server]).map_err(|e| e.to_string()).map(|_| ())
}

#[tauri::command]
#[specta::specta]
fn clear_all_command_history(state: tauri::State<'_, RuntimeState>) -> Result<(), String> { state.database.lock().map_err(|e| e.to_string())?.execute("DELETE FROM command_history", []).map_err(|e| e.to_string()).map(|_| ()) }

#[tauri::command]
#[specta::specta]
fn delete_command_history(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> { state.database.lock().map_err(|e| e.to_string())?.execute("DELETE FROM command_history WHERE id = ?1", [id]).map_err(|e| e.to_string()).map(|_| ()) }

#[tauri::command]
#[specta::specta]
fn set_server_capture(state: tauri::State<'_, RuntimeState>, server_id: String, enabled: bool) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_preferences (server_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 1)", []).map_err(|error| error.to_string())?;
    database.execute("INSERT INTO server_preferences (server_id, enabled) VALUES (?1, ?2) ON CONFLICT(server_id) DO UPDATE SET enabled = excluded.enabled", rusqlite::params![server_id, enabled as i64]).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn list_custom_commands(state: tauri::State<'_, RuntimeState>) -> Result<Vec<CustomCommandRecord>, String> { let db = state.database.lock().map_err(|e| e.to_string())?; let mut stmt = db.prepare("SELECT id,name,command,created_at,updated_at FROM custom_commands ORDER BY name").map_err(|e| e.to_string())?; let rows = stmt.query_map([], |r| Ok(CustomCommandRecord { id:r.get(0)?, name:r.get(1)?, command:r.get(2)?, created_at:r.get(3)?, updated_at:r.get(4)? })).map_err(|e|e.to_string())?; rows.map(|r|r.map_err(|e|e.to_string())).collect() }

#[tauri::command]
#[specta::specta]
fn create_custom_command(state: tauri::State<'_, RuntimeState>, input: String) -> Result<CustomCommandRecord, String> { let v:serde_json::Value=serde_json::from_str(&input).map_err(|e|e.to_string())?; let id=format!("command-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()); let now=utc_timestamp(); let r=CustomCommandRecord{id:id.clone(),name:v.get("name").and_then(|x|x.as_str()).unwrap_or_default().into(),command:v.get("command").and_then(|x|x.as_str()).unwrap_or_default().into(),created_at:now.clone(),updated_at:now}; state.database.lock().map_err(|e|e.to_string())?.execute("INSERT INTO custom_commands VALUES (?1,?2,?3,?4,?5)",[&r.id,&r.name,&r.command,&r.created_at,&r.updated_at]).map_err(|e|e.to_string())?; Ok(r) }

#[tauri::command]
#[specta::specta]
fn update_custom_command(state: tauri::State<'_, RuntimeState>, id: String, input: String) -> Result<CustomCommandRecord, String> { let v:serde_json::Value=serde_json::from_str(&input).map_err(|e|e.to_string())?; let db=state.database.lock().map_err(|e|e.to_string())?; db.execute("UPDATE custom_commands SET name=COALESCE(?1,name), command=COALESCE(?2,command), updated_at=?3 WHERE id=?4",rusqlite::params![v.get("name").and_then(|x|x.as_str()),v.get("command").and_then(|x|x.as_str()),utc_timestamp(),id]).map_err(|e|e.to_string())?; db.query_row("SELECT id,name,command,created_at,updated_at FROM custom_commands WHERE id=?1",[id],|r|Ok(CustomCommandRecord{id:r.get(0)?,name:r.get(1)?,command:r.get(2)?,created_at:r.get(3)?,updated_at:r.get(4)?})).map_err(|e|e.to_string()) }

#[tauri::command]
#[specta::specta]
fn delete_custom_command(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> { state.database.lock().map_err(|e|e.to_string())?.execute("DELETE FROM custom_commands WHERE id=?1",[id]).map_err(|e|e.to_string()).map(|_|()) }

#[tauri::command]
#[specta::specta]
fn list_sftp_bookmarks(state: tauri::State<'_, RuntimeState>, server_id: String) -> Result<Vec<SftpBookmarkRecord>, String> { let db=state.database.lock().map_err(|e|e.to_string())?; let mut s=db.prepare("SELECT id,server_id,path,created_at FROM sftp_bookmarks WHERE server_id=?1 ORDER BY path").map_err(|e|e.to_string())?; let rows=s.query_map([server_id],|r|Ok(SftpBookmarkRecord{id:r.get(0)?,server_id:r.get(1)?,path:r.get(2)?,created_at:r.get(3)?})).map_err(|e|e.to_string())?; rows.map(|r|r.map_err(|e|e.to_string())).collect() }

#[tauri::command]
#[specta::specta]
fn add_sftp_bookmark(state: tauri::State<'_, RuntimeState>, server_id: String, path: String) -> Result<SftpBookmarkRecord, String> { let id=format!("bookmark-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()); let r=SftpBookmarkRecord{id:id.clone(),server_id:server_id.clone(),path:path.clone(),created_at:utc_timestamp()}; state.database.lock().map_err(|e|e.to_string())?.execute("INSERT OR REPLACE INTO sftp_bookmarks (id,server_id,path,created_at) VALUES (?1,?2,?3,?4)",rusqlite::params![id,server_id,path,r.created_at]).map_err(|e|e.to_string())?; Ok(r) }

#[tauri::command]
#[specta::specta]
fn delete_sftp_bookmark(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> { state.database.lock().map_err(|e|e.to_string())?.execute("DELETE FROM sftp_bookmarks WHERE id=?1",[id]).map_err(|e|e.to_string()).map(|_|()) }

#[tauri::command]
#[specta::specta]
fn delete_sftp_bookmark_by_path(state: tauri::State<'_, RuntimeState>, server_id: String, path: String) -> Result<(), String> { state.database.lock().map_err(|e|e.to_string())?.execute("DELETE FROM sftp_bookmarks WHERE server_id=?1 AND path=?2",rusqlite::params![server_id,path]).map_err(|e|e.to_string()).map(|_|()) }

#[tauri::command]
#[specta::specta]
fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("settings.json");
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
#[specta::specta]
fn update_settings(app: AppHandle, input: String) -> Result<AppSettings, String> {
    let config_dir = app.path().app_config_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&config_dir).map_err(|error| error.to_string())?;
    let path = config_dir.join("settings.json");
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str::<AppSettings>(&contents).map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
        Err(error) => return Err(error.to_string()),
    };
    let patch = serde_json::from_str::<serde_json::Value>(&input).map_err(|error| error.to_string())?;
    if let Some(password) = patch.get("webdavPassword") {
        let data_dir = app.path().app_data_dir().map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
        let database = rusqlite::Connection::open(data_dir.join("russh.db")).map_err(|error| error.to_string())?;
        database.execute("CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)", []).map_err(|error| error.to_string())?;
        if password.is_null() || password.as_str().unwrap_or_default().is_empty() {
            database.execute("DELETE FROM app_settings WHERE key = 'webdav_backup_password'", []).map_err(|error| error.to_string())?;
        } else {
            database.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('webdav_backup_password', ?1)", [password.as_str().unwrap_or_default()]).map_err(|error| error.to_string())?;
        }
    }
    let target = serde_json::to_value(&mut settings).map_err(|error| error.to_string())?;
    let mut merged = target;
    let (Some(target), Some(patch)) = (merged.as_object_mut(), patch.as_object()) else {
        return Err("settings update must be a JSON object".into());
    };
    for (key, value) in patch {
        target.insert(key.clone(), value.clone());
    }
    settings = serde_json::from_value(merged).map_err(|error| error.to_string())?;
    let contents = serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, contents).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    Ok(settings)
}

fn webdav_settings(app: &AppHandle) -> Result<AppSettings, String> {
    get_settings(app.clone())
}

fn webdav_url(settings: &AppSettings) -> Result<String, String> {
    let base = settings.webdav_url.clone().ok_or("WebDAV URL is not configured")?;
    if !base.starts_with("http://") && !base.starts_with("https://") { return Err("WebDAV URL must use HTTP or HTTPS".into()); }
    Ok(format!("{}/{}", base.trim_end_matches('/'), settings.webdav_backup_path.clone().unwrap_or_else(|| "russh-backup".into()).trim_matches('/')))
}

fn webdav_file_url(settings: &AppSettings, file_name: &str) -> Result<String, String> {
    if file_name.is_empty() || file_name.contains('/') || file_name.contains('\\') || file_name == "." || file_name == ".." { return Err("invalid WebDAV backup file name".into()); }
    Ok(format!("{}/{}", webdav_url(settings)?, urlencoding::encode(file_name)))
}

fn webdav_client(_settings: &AppSettings) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|error| error.to_string())
}

fn webdav_request(app: &AppHandle, settings: &AppSettings, request: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
    let password = app.path().app_data_dir().ok().and_then(|path| rusqlite::Connection::open(path.join("russh.db")).ok()).and_then(|database| database.query_row("SELECT value FROM app_settings WHERE key = 'webdav_backup_password'", [], |row| row.get::<_, String>(0)).optional().ok().flatten());
    match (settings.webdav_username.as_deref(), password.as_deref()) { (Some(username), Some(password)) if !username.is_empty() => request.basic_auth(username, Some(password)), _ => request }
}

#[tauri::command]
#[specta::specta]
fn test_webdav_connection(app: AppHandle) -> Result<WebDavTestRecord, String> {
    let settings = webdav_settings(&app)?;
    let url = webdav_url(&settings)?;
    let client = webdav_client(&settings)?;
    let response = webdav_request(&app, &settings, client.request(reqwest::Method::from_bytes(b"PROPFIND").map_err(|error| error.to_string())?, &url).header("Depth", "0").body(String::new())).send().map_err(|error| error.to_string())?;
    let status = response.status();
    Ok(WebDavTestRecord { ok: status.is_success() || status.as_u16() == 207, message: if status.is_success() || status.as_u16() == 207 { "WebDAV connection is available".into() } else { "WebDAV server rejected the request".into() } })
}

#[tauri::command]
#[specta::specta]
fn list_webdav_backups(app: AppHandle) -> Result<Vec<WebDavBackupRecord>, String> {
    let settings = webdav_settings(&app)?;
    let url = webdav_url(&settings)?;
    let client = webdav_client(&settings)?;
    let response = webdav_request(&app, &settings, client.request(reqwest::Method::from_bytes(b"PROPFIND").map_err(|error| error.to_string())?, &url).header("Depth", "1").body(String::new())).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() && response.status().as_u16() != 207 { return Err(format!("WebDAV list failed with HTTP {}", response.status())); }
    let body = response.text().map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for href in body.split("<d:href>").skip(1).filter_map(|part| part.split("</d:href>").next()) {
        let file_name = href.rsplit('/').next().unwrap_or_default();
        if file_name.starts_with("russh-") && file_name.ends_with(".db") { results.push(WebDavBackupRecord { file_name: file_name.into(), modified_at: String::new() }); }
    }
    Ok(results)
}

#[tauri::command]
#[specta::specta]
fn backup_webdav_now(app: AppHandle) -> Result<(), String> {
    let settings = webdav_settings(&app)?;
    let database_path = app.path().app_data_dir().map_err(|error| error.to_string())?.join("russh.db");
    let body = std::fs::read(&database_path).map_err(|error| error.to_string())?;
    let file_name = format!("russh-{}-{}.db", std::env::consts::OS, utc_timestamp());
    let response = webdav_request(&app, &settings, webdav_client(&settings)?.put(webdav_file_url(&settings, &file_name)?).body(body)).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() && response.status().as_u16() != 201 { return Err(format!("WebDAV upload failed with HTTP {}", response.status())); }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn delete_webdav_backup(app: AppHandle, file_name: String) -> Result<(), String> {
    let settings = webdav_settings(&app)?;
    let response = webdav_request(&app, &settings, webdav_client(&settings)?.delete(webdav_file_url(&settings, &file_name)?)).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() && response.status().as_u16() != 204 { return Err(format!("WebDAV delete failed with HTTP {}", response.status())); }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn restore_webdav_backup(app: AppHandle, file_name: String) -> Result<(), String> {
    let settings = webdav_settings(&app)?;
    let response = webdav_request(&app, &settings, webdav_client(&settings)?.get(webdav_file_url(&settings, &file_name)?)).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() { return Err(format!("WebDAV restore download failed with HTTP {}", response.status())); }
    let bytes = response.bytes().map_err(|error| error.to_string())?;
    if bytes.len() < 16 || &bytes[..16] != b"SQLite format 3\0" { return Err("the selected WebDAV backup is not a valid SQLite database".into()); }
    let data_dir = app.path().app_data_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    let target = data_dir.join("russh.db");
    let temporary = data_dir.join(format!(".russh-restore-{}.db", utc_timestamp()));
    std::fs::write(&temporary, &bytes).map_err(|error| error.to_string())?;
    let backup = data_dir.join(format!("russh.db.before-restore-{}", utc_timestamp()));
    if target.exists() { std::fs::rename(&target, &backup).map_err(|error| error.to_string())?; }
    if let Err(error) = std::fs::rename(&temporary, &target) {
        if backup.exists() { let _ = std::fs::rename(&backup, &target); }
        return Err(error.to_string());
    }
    app.restart();
}

fn utc_timestamp() -> String {
    format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
}

fn normalize_entered_command(input: &str) -> String {
    let mut output = String::new();
    let mut escape = false;
    for character in input.chars() {
        if escape {
            if character.is_ascii_alphabetic() || character == '~' { escape = false; }
            continue;
        }
        match character {
            '\u{1b}' => escape = true,
            '\u{7f}' | '\u{8}' => { output.pop(); }
            character if character.is_control() && character != '\t' => {}
            character => output.push(character),
        }
    }
    output.trim().to_owned()
}

fn host_fingerprint(session: &ssh2::Session) -> Result<String, String> {
    let hash = session.host_key_hash(ssh2::HashType::Sha256).ok_or("SSH host key unavailable")?;
    Ok(format!("SHA256:{}", STANDARD.encode(hash)))
}

fn authenticate_session(session: &mut ssh2::Session, info: &SessionConnectionInfo, tag: &str) -> Result<(), String> {
    if let Some(private_key) = info.private_key.as_deref() {
        let key_path = std::env::temp_dir().join(format!("russh-key-{tag}"));
        std::fs::write(&key_path, private_key).map_err(|error| error.to_string())?;
        let result = session.userauth_pubkey_file(&info.username, None, &key_path, info.passphrase.as_deref()).map_err(|error| error.to_string());
        let _ = std::fs::remove_file(key_path);
        result
    } else {
        session.userauth_password(&info.username, &info.password).map_err(|error| error.to_string())
    }
}

fn load_connection_info(
    database: &Connection,
    server_id: &str,
    request: &serde_json::Value,
) -> Result<(String, String, u16, String, Option<String>, SessionConnectionInfo), String> {
    let (name, host, port, username, jump_server_id): (String, String, u16, String, Option<String>) = database
        .query_row(
            "SELECT name, host, port, username, jump_server_id FROM servers WHERE id = ?1",
            [server_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|error| error.to_string())?;
    let secrets: (Option<String>, Option<String>, Option<String>) = database
        .query_row(
            "SELECT password, private_key, passphrase FROM server_secrets WHERE server_id = ?1",
            [server_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or((None, None, None));
    let request_secrets = request.get("secrets").and_then(|value| value.get(server_id));
    let password = request_secrets
        .and_then(|value| value.get("password"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .or(secrets.0);
    let private_key = request_secrets
        .and_then(|value| value.get("privateKey"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .or(secrets.1);
    let passphrase = request_secrets
        .and_then(|value| value.get("passphrase"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .or(secrets.2);
    let info = SessionConnectionInfo {
        server_id: server_id.to_owned(),
        host: host.clone(),
        port,
        username: username.clone(),
        password: password.unwrap_or_default(),
        private_key,
        passphrase,
        jump: None,
    };
    Ok((name, host, port, username, jump_server_id, info))
}

fn connect_through_jump(
    jump: &SessionConnectionInfo,
    target_host: &str,
    target_port: u16,
    tag: &str,
) -> Result<TcpStream, String> {
    let jump_tcp = TcpStream::connect((jump.host.as_str(), jump.port)).map_err(|error| {
        format!("jump host {}:{} connection failed: {error}", jump.host, jump.port)
    })?;
    let mut jump_session = ssh2::Session::new().map_err(|error| error.to_string())?;
    jump_session.set_timeout(30_000);
    jump_session.set_tcp_stream(jump_tcp);
    jump_session.handshake().map_err(|error| {
        format!("jump host {}:{} SSH handshake failed: {error}", jump.host, jump.port)
    })?;
    authenticate_session(&mut jump_session, jump, &format!("{tag}-jump")).map_err(|error| {
        format!("jump host {}:{} authentication failed: {error}", jump.host, jump.port)
    })?;
    let mut channel = jump_session
        .channel_direct_tcpip(target_host, target_port, None)
        .map_err(|error| {
            format!(
                "jump host {}:{} cannot forward to target {}:{}: {error}",
                jump.host, jump.port, target_host, target_port
            )
        })?;
    // The two relay directions share libssh2's session lock. Keep channel I/O
    // non-blocking so a read waiting for target output cannot prevent the
    // opposite direction from forwarding the client's SSH banner.
    jump_session.set_blocking(false);

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let target = TcpStream::connect(address).map_err(|error| error.to_string())?;
    target.set_nodelay(true).map_err(|error| error.to_string())?;
    let Ok((local_for_read, _)) = listener.accept() else {
        return Err("jump relay listener closed before target connected".into());
    };
    // `target` is the client side consumed by libssh2. Both relay directions
    // must use clones of the accepted server side; cloning `target` here
    // sends remote output back into the relay instead of into libssh2.
    // This is the accepted relay-side socket. It is distinct from `target`,
    // which is owned by libssh2, so non-blocking mode here cannot alter the
    // target SSH session's socket.
    local_for_read.set_nonblocking(true).map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let mut local = local_for_read;
        let mut to_jump = Vec::new();
        let mut from_jump = Vec::new();
        let mut local_buffer = [0_u8; 64 * 1024];
        let mut jump_buffer = [0_u8; 64 * 1024];
        loop {
            if to_jump.len() < 256 * 1024 {
                match local.read(&mut local_buffer) {
                    Ok(0) => break,
                    Ok(size) => to_jump.extend_from_slice(&local_buffer[..size]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_error) => {
                        break;
                    }
                }
            }
            if from_jump.len() < 256 * 1024 {
                match channel.read(&mut jump_buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        from_jump.extend_from_slice(&jump_buffer[..size]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_error) => {
                        break;
                    }
                }
            }
            if !to_jump.is_empty() {
                match channel.write(&to_jump) {
                    Ok(size) if size > 0 => {
                        to_jump.drain(..size);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_error) => {
                        break;
                    }
                    Ok(_) => {}
                }
            }
            if !from_jump.is_empty() {
                match local.write(&from_jump) {
                    Ok(size) if size > 0 => {
                        from_jump.drain(..size);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_error) => {
                        break;
                    }
                    Ok(_) => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    });

    Ok(target)
}

fn extract_cwd(data: &str) -> Option<String> {
    if let Some(start) = data.find("\u{1b}]7;file://") {
        let value = &data[start + "\u{1b}]7;file://".len()..];
        let value = value.split(['\u{7}', '\u{1b}']).next().unwrap_or_default();
        return value.find('/').map(|index| value[index..].to_owned());
    }
    data.find("\u{1b}]133;P;Cwd=").map(|start| {
        let value = &data[start + "\u{1b}]133;P;Cwd=".len()..];
        value.split(['\u{7}', '\u{1b}']).next().unwrap_or_default().to_owned()
    }).filter(|value| !value.is_empty())
}

#[tauri::command]
#[specta::specta]
fn list_groups(state: tauri::State<'_, RuntimeState>) -> Result<Vec<GroupRecord>, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    let mut statement = database
        .prepare("SELECT id, name, parent_id, color, created_at, updated_at FROM groups ORDER BY name")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(GroupRecord {
                id: row.get(0)?, name: row.get(1)?, parent_id: row.get(2)?,
                color: row.get(3)?, created_at: row.get(4)?, updated_at: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn create_group(state: tauri::State<'_, RuntimeState>, input: String) -> Result<GroupRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let name = value.get("name").and_then(|value| value.as_str()).ok_or("group name is required")?;
    let color = value.get("color").and_then(|value| value.as_str()).unwrap_or("#808080");
    let parent_id = value.get("parentId").and_then(|value| value.as_str());
    let now = utc_timestamp();
    let record = GroupRecord { id: format!("group-{now}"), name: name.into(), parent_id: parent_id.map(str::to_owned), color: color.into(), created_at: now.clone(), updated_at: now };
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("INSERT INTO groups (id, name, parent_id, color, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![record.id, record.name, record.parent_id, record.color, record.created_at, record.updated_at]).map_err(|error| error.to_string())?;
    Ok(record)
}

#[tauri::command]
#[specta::specta]
fn delete_group(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("DELETE FROM groups WHERE id = ?1", [id]).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn update_group(state: tauri::State<'_, RuntimeState>, id: String, input: String) -> Result<GroupRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("UPDATE groups SET name = COALESCE(?1, name), color = COALESCE(?2, color), parent_id = ?3, updated_at = ?4 WHERE id = ?5", rusqlite::params![value.get("name").and_then(|value| value.as_str()), value.get("color").and_then(|value| value.as_str()), value.get("parentId").and_then(|value| value.as_str()), utc_timestamp(), id]).map_err(|error| error.to_string())?;
    database.query_row("SELECT id, name, parent_id, color, created_at, updated_at FROM groups WHERE id = ?1", [id], |row| Ok(GroupRecord { id: row.get(0)?, name: row.get(1)?, parent_id: row.get(2)?, color: row.get(3)?, created_at: row.get(4)?, updated_at: row.get(5)? })).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_tags(state: tauri::State<'_, RuntimeState>) -> Result<Vec<TagRecord>, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    let mut statement = database
        .prepare("SELECT id, name, color, created_at, updated_at FROM tags ORDER BY name")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok(TagRecord {
            id: row.get(0)?, name: row.get(1)?, color: row.get(2)?,
            created_at: row.get(3)?, updated_at: row.get(4)?,
        }))
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn create_tag(state: tauri::State<'_, RuntimeState>, input: String) -> Result<TagRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let name = value.get("name").and_then(|value| value.as_str()).ok_or("tag name is required")?;
    let color = value.get("color").and_then(|value| value.as_str()).unwrap_or("#808080");
    let now = utc_timestamp();
    let record = TagRecord { id: format!("tag-{now}"), name: name.into(), color: color.into(), created_at: now.clone(), updated_at: now };
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("INSERT INTO tags (id, name, color, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![record.id, record.name, record.color, record.created_at, record.updated_at]).map_err(|error| error.to_string())?;
    Ok(record)
}

#[tauri::command]
#[specta::specta]
fn delete_tag(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("DELETE FROM tags WHERE id = ?1", [id]).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn update_tag(state: tauri::State<'_, RuntimeState>, id: String, input: String) -> Result<TagRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("UPDATE tags SET name = COALESCE(?1, name), color = COALESCE(?2, color), updated_at = ?3 WHERE id = ?4", rusqlite::params![value.get("name").and_then(|value| value.as_str()), value.get("color").and_then(|value| value.as_str()), utc_timestamp(), id]).map_err(|error| error.to_string())?;
    database.query_row("SELECT id, name, color, created_at, updated_at FROM tags WHERE id = ?1", [id], |row| Ok(TagRecord { id: row.get(0)?, name: row.get(1)?, color: row.get(2)?, created_at: row.get(3)?, updated_at: row.get(4)? })).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_servers(state: tauri::State<'_, RuntimeState>) -> Result<String, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_preferences (server_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 1)", []).map_err(|error| error.to_string())?;
    let mut statement = database.prepare("SELECT id, name, host, port, username, auth_type, favorite, created_at, updated_at, jump_server_id, group_id FROM servers ORDER BY favorite DESC, name").map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok(serde_json::json!({
        "id": row.get::<_, String>(0)?, "name": row.get::<_, String>(1)?, "host": row.get::<_, String>(2)?,
        "port": row.get::<_, i64>(3)?, "username": row.get::<_, String>(4)?, "authType": row.get::<_, String>(5)?,
        "favorite": row.get::<_, i64>(6)? != 0, "createdAt": row.get::<_, String>(7)?, "updatedAt": row.get::<_, String>(8)?,
        "brandId": null, "customIconDataUrl": null, "privateKeyPath": null, "note": null,
        "credentialId": null, "jumpServerId": row.get::<_, Option<String>>(9)?, "groupId": row.get::<_, Option<String>>(10)?, "proxyMode": "none", "proxyType": "socks5", "proxyHost": null,
        "proxyPort": 1080, "captureCommandHistory": true, "lastConnectedAt": null, "group": null, "tags": [],
        "hasPassword": false, "hasPassphrase": false
    }))).map_err(|error| error.to_string())?;
    let values: Result<Vec<_>, _> = rows.map(|row| row.map_err(|error| error.to_string())).collect();
    let mut values = values?;
    for value in &mut values {
        if let Some(id) = value.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
            let secret = database.query_row("SELECT password IS NOT NULL, passphrase IS NOT NULL FROM server_secrets WHERE server_id = ?1", [id.clone()], |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?))).optional().map_err(|error| error.to_string())?.unwrap_or((false, false));
            value["hasPassword"] = serde_json::Value::Bool(secret.0);
            value["hasPassphrase"] = serde_json::Value::Bool(secret.1);
            let capture = database.query_row("SELECT enabled != 0 FROM server_preferences WHERE server_id = ?1", [id], |row| row.get::<_, bool>(0)).optional().map_err(|error| error.to_string())?.unwrap_or(true);
            value["captureCommandHistory"] = serde_json::Value::Bool(capture);
        }
    }
    serde_json::to_string(&values).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_recent_servers(state: tauri::State<'_, RuntimeState>) -> Result<Vec<RecentServerRecord>, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    let mut statement = database.prepare("SELECT recent_sessions.id, recent_sessions.server_id, recent_sessions.connected_at, servers.name, servers.host FROM recent_sessions INNER JOIN servers ON servers.id = recent_sessions.server_id ORDER BY recent_sessions.connected_at DESC LIMIT 8").map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok(RecentServerRecord { id: row.get(0)?, server_id: row.get(1)?, connected_at: row.get(2)?, server_name: row.get(3)?, host: row.get(4)? })).map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn clear_recent_servers(state: tauri::State<'_, RuntimeState>) -> Result<(), String> {
    state.database.lock().map_err(|error| error.to_string())?.execute("DELETE FROM recent_sessions", []).map_err(|error| error.to_string()).map(|_| ())
}

#[tauri::command]
#[specta::specta]
fn create_server(state: tauri::State<'_, RuntimeState>, input: String) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let name = value.get("name").and_then(|value| value.as_str()).ok_or("server name is required")?;
    let host = value.get("host").and_then(|value| value.as_str()).ok_or("server host is required")?;
    let username = value.get("username").and_then(|value| value.as_str()).unwrap_or("");
    let port = value.get("port").and_then(|value| value.as_i64()).unwrap_or(22);
    let auth_type = value.get("authType").and_then(|value| value.as_str()).unwrap_or("password");
    let id = value.get("id").and_then(|value| value.as_str()).map(str::to_owned).unwrap_or_else(|| format!("server-{}", utc_timestamp()));
    let now = utc_timestamp();
    let favorite = value.get("favorite").and_then(|value| value.as_bool()).unwrap_or(false);
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    database.execute("INSERT INTO servers (id, name, host, port, username, auth_type, favorite, jump_server_id, group_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", rusqlite::params![id, name, host, port, username, auth_type, favorite as i64, value.get("jumpServerId").and_then(|v| v.as_str()), value.get("groupId").and_then(|v| v.as_str()), now, now]).map_err(|error| error.to_string())?;
    database.execute("INSERT OR REPLACE INTO server_secrets (server_id, password, private_key, passphrase) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![id, value.get("password").and_then(|v| v.as_str()), value.get("privateKey").and_then(|v| v.as_str()), value.get("passphrase").and_then(|v| v.as_str())]).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"id": id, "name": name, "host": host, "port": port, "username": username, "authType": auth_type, "favorite": favorite}).to_string())
}

#[tauri::command]
#[specta::specta]
fn delete_server(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("DELETE FROM servers WHERE id = ?1", [id.clone()]).map_err(|error| error.to_string())?;
    let _ = database.execute("DELETE FROM server_secrets WHERE server_id = ?1", [id]);
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn update_server(state: tauri::State<'_, RuntimeState>, id: String, input: String) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    let name = value.get("name").and_then(|value| value.as_str());
    let host = value.get("host").and_then(|value| value.as_str());
    let username = value.get("username").and_then(|value| value.as_str());
    let port = value.get("port").and_then(|value| value.as_i64());
    let auth_type = value.get("authType").and_then(|value| value.as_str());
    let favorite = value.get("favorite").and_then(|value| value.as_bool()).map(|value| value as i64);
    let jump_server_id = value.get("jumpServerId").and_then(|value| value.as_str());
    let group_id = value.get("groupId").and_then(|value| value.as_str());
    let has_jump_server_id = value.get("jumpServerId").is_some();
    let has_group_id = value.get("groupId").is_some();
    database.execute("UPDATE servers SET name = COALESCE(?1, name), host = COALESCE(?2, host), port = COALESCE(?3, port), username = COALESCE(?4, username), auth_type = COALESCE(?5, auth_type), favorite = COALESCE(?6, favorite), jump_server_id = CASE WHEN ?7 THEN ?8 ELSE jump_server_id END, group_id = CASE WHEN ?9 THEN ?10 ELSE group_id END, updated_at = ?11 WHERE id = ?12", rusqlite::params![name, host, port, username, auth_type, favorite, has_jump_server_id, jump_server_id, has_group_id, group_id, utc_timestamp(), id]).map_err(|error| error.to_string())?;
    database.execute("INSERT INTO server_secrets (server_id, password, private_key, passphrase) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(server_id) DO UPDATE SET password=COALESCE(excluded.password, server_secrets.password), private_key=COALESCE(excluded.private_key, server_secrets.private_key), passphrase=COALESCE(excluded.passphrase, server_secrets.passphrase)", rusqlite::params![id, value.get("password").and_then(|v| v.as_str()), value.get("privateKey").and_then(|v| v.as_str()), value.get("passphrase").and_then(|v| v.as_str())]).map_err(|error| error.to_string())?;
    let mut statement = database.prepare("SELECT id, name, host, port, username, auth_type, favorite, created_at, updated_at, jump_server_id, group_id FROM servers WHERE id = ?1").map_err(|error| error.to_string())?;
    let record = statement.query_row([id], |row| Ok(serde_json::json!({
        "id": row.get::<_, String>(0)?, "name": row.get::<_, String>(1)?, "host": row.get::<_, String>(2)?,
        "port": row.get::<_, i64>(3)?, "username": row.get::<_, String>(4)?, "authType": row.get::<_, String>(5)?,
        "favorite": row.get::<_, i64>(6)? != 0, "createdAt": row.get::<_, String>(7)?, "updatedAt": row.get::<_, String>(8)?, "jumpServerId": row.get::<_, Option<String>>(9)?, "groupId": row.get::<_, Option<String>>(10)?
    }))).map_err(|error| error.to_string())?;
    serde_json::to_string(&record).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn get_server_secrets(state: tauri::State<'_, RuntimeState>, id: String) -> Result<String, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    let record = database.query_row("SELECT password, private_key, passphrase FROM server_secrets WHERE server_id = ?1", [id], |row| Ok(serde_json::json!({"password": row.get::<_, Option<String>>(0)?, "privateKey": row.get::<_, Option<String>>(1)?, "passphrase": row.get::<_, Option<String>>(2)?}))).optional().map_err(|error| error.to_string())?.unwrap_or_else(|| serde_json::json!({"password": null, "privateKey": null, "passphrase": null}));
    serde_json::to_string(&record).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
async fn export_server_config(app: AppHandle, state: tauri::State<'_, RuntimeState>, server_id: Option<String>) -> Result<(), String> {
    let servers: Vec<serde_json::Value> = {
        let database = state.database.lock().map_err(|error| error.to_string())?;
        database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
        let mut statement = database.prepare("SELECT id, name, host, port, username, auth_type, favorite, jump_server_id, group_id FROM servers WHERE (?1 IS NULL OR id = ?1) ORDER BY name").map_err(|error| error.to_string())?;
        let rows = statement.query_map([server_id.as_deref()], |row| {
            let id: String = row.get(0)?;
            let secret = database.query_row("SELECT password, private_key, passphrase FROM server_secrets WHERE server_id = ?1", [&id], |secret_row| Ok(serde_json::json!({
                "password": secret_row.get::<_, Option<String>>(0)?, "privateKey": secret_row.get::<_, Option<String>>(1)?, "passphrase": secret_row.get::<_, Option<String>>(2)?
            }))).optional()?.unwrap_or_else(|| serde_json::json!({"password": null, "privateKey": null, "passphrase": null}));
            Ok(serde_json::json!({
                "name": row.get::<_, String>(1)?, "host": row.get::<_, String>(2)?, "port": row.get::<_, i64>(3)?, "username": row.get::<_, String>(4)?,
                "authType": row.get::<_, String>(5)?, "favorite": row.get::<_, i64>(6)? != 0, "jumpServerId": row.get::<_, Option<String>>(7)?, "secrets": secret
            }))
        }).map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string())).collect::<Result<_, _>>()?
    };
    let content = servers.iter().map(|server| {
        let json = serde_json::to_string(server).map_err(|error| error.to_string())?;
        Ok::<String, String>(STANDARD.encode(json.as_bytes()))
    }).collect::<Result<Vec<_>, _>>()?.join("\n");
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().add_filter("RuSSH Config", &["russh"]).set_file_name("russh-servers.russh").save_file(move |path| { let _ = sender.send(path); });
    if let Some(path) = receiver.await.map_err(|error| error.to_string())? { std::fs::write(path.into_path().map_err(|error| error.to_string())?, content).map_err(|error| error.to_string())?; }
    Ok(())
}

#[tauri::command]
#[specta::specta]
async fn import_server_config(app: AppHandle, state: tauri::State<'_, RuntimeState>) -> Result<u32, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().add_filter("RuSSH Config", &["russh"]).pick_file(move |path| { let _ = sender.send(path); });
    let Some(path) = receiver.await.map_err(|error| error.to_string())? else { return Ok(0); };
    let content = std::fs::read_to_string(path.into_path().map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let items: Vec<serde_json::Value> = content.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()).map(|(line_number, line)| {
        let decoded = STANDARD.decode(line.trim()).map_err(|error| format!("invalid base64 at line {}: {}", line_number + 1, error))?;
        serde_json::from_slice(&decoded).map_err(|error| format!("invalid server JSON at line {}: {}", line_number + 1, error))
    }).collect::<Result<_, _>>()?;
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    let mut imported = 0u32;
    for (index, item) in items.iter().enumerate() {
        let name = item.get("name").and_then(|v| v.as_str()).ok_or("server name is required")?;
        let host = item.get("host").and_then(|v| v.as_str()).ok_or("server host is required")?;
        let id = format!("server-import-{}-{}", utc_timestamp(), index);
        let now = utc_timestamp();
        let group_id = item.get("groupId").and_then(|v| v.as_str());
        let group_id = group_id.filter(|id| database.query_row("SELECT 1 FROM groups WHERE id = ?1", [id], |_| Ok(())).optional().ok().flatten().is_some());
        database.execute("INSERT INTO servers (id, name, host, port, username, auth_type, favorite, jump_server_id, group_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", rusqlite::params![id, name, host, item.get("port").and_then(|v| v.as_i64()).unwrap_or(22), item.get("username").and_then(|v| v.as_str()).unwrap_or(""), item.get("authType").and_then(|v| v.as_str()).unwrap_or("password"), item.get("favorite").and_then(|v| v.as_bool()).unwrap_or(false) as i64, Option::<&str>::None, group_id, now, now]).map_err(|error| error.to_string())?;
        let secrets = item.get("secrets");
        database.execute("INSERT INTO server_secrets (server_id, password, private_key, passphrase) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![id, secrets.and_then(|v| v.get("password")).and_then(|v| v.as_str()), secrets.and_then(|v| v.get("privateKey")).and_then(|v| v.as_str()), secrets.and_then(|v| v.get("passphrase")).and_then(|v| v.as_str())]).map_err(|error| error.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

#[tauri::command]
#[specta::specta]
fn toggle_server_favorite(state: tauri::State<'_, RuntimeState>, id: String) -> Result<String, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("UPDATE servers SET favorite = CASE favorite WHEN 0 THEN 1 ELSE 0 END, updated_at = ?1 WHERE id = ?2", rusqlite::params![utc_timestamp(), id]).map_err(|error| error.to_string())?;
    let favorite: bool = database.query_row("SELECT favorite != 0 FROM servers WHERE id = ?1", [id], |row| row.get(0)).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"favorite": favorite}).to_string())
}

#[tauri::command]
#[specta::specta]
fn list_credentials(state: tauri::State<'_, RuntimeState>) -> Result<Vec<CredentialRecord>, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    let mut statement = database.prepare("SELECT id, name, kind, username, note, created_at, updated_at FROM credentials ORDER BY name").map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok(CredentialRecord {
        id: row.get(0)?, name: row.get(1)?, kind: row.get(2)?, username: row.get(3)?, note: row.get(4)?, created_at: row.get(5)?, updated_at: row.get(6)?,
    })).map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn get_credential_secret(state: tauri::State<'_, RuntimeState>, id: String) -> Result<CredentialSecretRecord, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.query_row("SELECT password, private_key, passphrase FROM credentials WHERE id = ?1", [id], |row| Ok(CredentialSecretRecord { password: row.get(0)?, private_key: row.get(1)?, passphrase: row.get(2)? })).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn create_credential(state: tauri::State<'_, RuntimeState>, input: String) -> Result<CredentialRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let name = value.get("name").and_then(|value| value.as_str()).ok_or("credential name is required")?;
    let kind = value.get("kind").and_then(|value| value.as_str()).unwrap_or("password");
    let id = format!("credential-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let now = utc_timestamp();
    let record = CredentialRecord { id: id.clone(), name: name.into(), kind: kind.into(), username: value.get("username").and_then(|value| value.as_str()).map(str::to_owned), note: value.get("note").and_then(|value| value.as_str()).map(str::to_owned), created_at: now.clone(), updated_at: now.clone() };
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("INSERT INTO credentials (id, name, kind, username, password, private_key, passphrase, note, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", rusqlite::params![id, name, kind, record.username, value.get("password").and_then(|value| value.as_str()), value.get("privateKey").and_then(|value| value.as_str()), value.get("passphrase").and_then(|value| value.as_str()), record.note, record.created_at, record.updated_at]).map_err(|error| error.to_string())?;
    Ok(record)
}

#[tauri::command]
#[specta::specta]
fn delete_credential(state: tauri::State<'_, RuntimeState>, id: String) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("DELETE FROM credentials WHERE id = ?1", [id]).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn update_credential(state: tauri::State<'_, RuntimeState>, id: String, input: String) -> Result<CredentialRecord, String> {
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("UPDATE credentials SET name = COALESCE(?1, name), kind = COALESCE(?2, kind), username = ?3, password = COALESCE(?4, password), private_key = COALESCE(?5, private_key), passphrase = COALESCE(?6, passphrase), note = ?7, updated_at = ?8 WHERE id = ?9", rusqlite::params![value.get("name").and_then(|value| value.as_str()), value.get("kind").and_then(|value| value.as_str()), value.get("username").and_then(|value| value.as_str()), value.get("password").and_then(|value| value.as_str()), value.get("privateKey").and_then(|value| value.as_str()), value.get("passphrase").and_then(|value| value.as_str()), value.get("note").and_then(|value| value.as_str()), utc_timestamp(), id]).map_err(|error| error.to_string())?;
    database.query_row("SELECT id, name, kind, username, note, created_at, updated_at FROM credentials WHERE id = ?1", [id], |row| Ok(CredentialRecord { id: row.get(0)?, name: row.get(1)?, kind: row.get(2)?, username: row.get(3)?, note: row.get(4)?, created_at: row.get(5)?, updated_at: row.get(6)? })).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn database_health(state: tauri::State<'_, RuntimeState>) -> Result<String, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database
        .query_row("SELECT value FROM schema_meta WHERE key = 'schema_version'", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_known_hosts(state: tauri::State<'_, RuntimeState>) -> Result<Vec<KnownHostRecord>, String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    let mut statement = database.prepare("SELECT host, port, algorithm, fingerprint, verified_at FROM known_hosts ORDER BY verified_at DESC, host, port").map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok(KnownHostRecord { host: row.get(0)?, port: row.get(1)?, algorithm: row.get(2)?, fingerprint: row.get(3)?, verified_at: row.get(4)? })).map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string())).collect()
}

#[tauri::command]
#[specta::specta]
fn remove_known_host(state: tauri::State<'_, RuntimeState>, host: String, port: u16) -> Result<(), String> {
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("DELETE FROM known_hosts WHERE host = ?1 AND port = ?2", rusqlite::params![host, port]).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn respond_host_trust(state: tauri::State<'_, RuntimeState>, result: String) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(&result).map_err(|error| error.to_string())?;
    let request_id = value.get("requestId").and_then(|value| value.as_str()).ok_or("requestId is required")?;
    let trusted = value.get("trusted").and_then(|value| value.as_bool()).unwrap_or(false);
    if let Some(sender) = state.pending_trust.lock().map_err(|error| error.to_string())?.remove(request_id) { sender.send(trusted).map_err(|error| error.to_string())?; }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn create_local_terminal(app: AppHandle, state: tauri::State<'_, RuntimeState>) -> Result<LocalTerminalSummary, String> {
    let terminal_id = format!("terminal-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let shell = if cfg!(target_os = "windows") { std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()) } else { std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()) };
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?.to_string_lossy().into_owned();
    let pty = native_pty_system().openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }).map_err(|error| error.to_string())?;
    let mut command = CommandBuilder::new(&shell);
    command.cwd(&cwd);
    let child = pty.slave.spawn_command(command).map_err(|error| error.to_string())?;
    let mut stdout = pty.master.try_clone_reader().map_err(|error| error.to_string())?;
    let writer = pty.master.take_writer().map_err(|error| error.to_string())?;
    let event_app = app.clone();
    let event_id = terminal_id.clone();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(size) => {
                    let data = String::from_utf8_lossy(&buffer[..size]).into_owned();
                    let _ = event_app.emit("localTerminals:data", serde_json::json!({"terminalId": event_id, "data": data}));
                }
            }
        }
        let _ = event_app.emit("localTerminals:state", serde_json::json!({"terminalId": event_id, "status": "stopped"}));
        let _ = event_app.emit("localTerminals:exit", serde_json::json!({"terminalId": event_id, "exitCode": 0}));
    });
    let summary = LocalTerminalSummary { terminal_id: terminal_id.clone(), title: shell.clone(), shell, cwd, started_at: utc_timestamp(), status: "running".into() };
    state.local_terminals.lock().map_err(|error| error.to_string())?.insert(terminal_id, LocalTerminalProcess { child, master: pty.master, writer });
    let _ = app.emit("localTerminals:state", serde_json::json!({"terminalId": summary.terminal_id, "status": "running"}));
    Ok(summary)
}

#[tauri::command]
#[specta::specta]
fn write_local_terminal(state: tauri::State<'_, RuntimeState>, terminal_id: String, data: String) -> Result<(), String> {
    let mut terminals = state.local_terminals.lock().map_err(|error| error.to_string())?;
    let terminal = terminals.get_mut(&terminal_id).ok_or("local terminal not found")?;
    terminal.writer.write_all(data.as_bytes()).map_err(|error| error.to_string())?;
    terminal.writer.flush().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn resize_local_terminal(state: tauri::State<'_, RuntimeState>, terminal_id: String, columns: u16, rows: u16) -> Result<(), String> {
    let terminals = state.local_terminals.lock().map_err(|error| error.to_string())?;
    let terminal = terminals.get(&terminal_id).ok_or("local terminal not found")?;
    terminal.master.resize(PtySize { rows, cols: columns, pixel_width: 0, pixel_height: 0 }).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn close_local_terminal(app: AppHandle, state: tauri::State<'_, RuntimeState>, terminal_id: String) -> Result<(), String> {
    let mut terminals = state.local_terminals.lock().map_err(|error| error.to_string())?;
    if let Some(mut terminal) = terminals.remove(&terminal_id) {
        let _ = terminal.child.kill();
        let _ = app.emit("localTerminals:state", serde_json::json!({"terminalId": terminal_id, "status": "stopped"}));
        let _ = app.emit("localTerminals:exit", serde_json::json!({"terminalId": terminal_id, "exitCode": 0}));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn connect_session(app: AppHandle, state: tauri::State<'_, RuntimeState>, input: String) -> Result<String, String> {
    let request: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let server_id = request.get("serverId").and_then(|value| value.as_str()).ok_or("serverId is required")?.to_owned();
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS server_secrets (server_id TEXT PRIMARY KEY, password TEXT, private_key TEXT, passphrase TEXT)", []).map_err(|error| error.to_string())?;
    let (name, host, port, _username, jump_server_id, target_info) = load_connection_info(&database, &server_id, &request)?;
    let known_fingerprint: Option<String> = database.query_row("SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2", rusqlite::params![host, port], |row| row.get(0)).optional().map_err(|error| error.to_string())?;
    drop(database);
    let connection_username = target_info.username.clone();
    let connection_password = (!target_info.password.is_empty()).then(|| target_info.password.clone());
    let connection_private_key = target_info.private_key.clone();
    let connection_passphrase = target_info.passphrase.clone();
    let expected_fingerprint = known_fingerprint.clone();
    let trust_server_name = name.clone();
    let pending_trust = state.pending_trust.clone();
    let session_id = request.get("sessionId").and_then(|value| value.as_str()).map(str::to_owned).unwrap_or_else(|| format!("session-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
    let connected_at = utc_timestamp();
    let recent_id = format!("recent-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let database = state.database.lock().map_err(|error| error.to_string())?;
    database.execute("CREATE TABLE IF NOT EXISTS recent_sessions (id TEXT PRIMARY KEY, server_id TEXT NOT NULL, connected_at TEXT NOT NULL)", []).map_err(|error| error.to_string())?;
    database.execute("INSERT INTO recent_sessions (id, server_id, connected_at) VALUES (?1, ?2, ?3)", rusqlite::params![recent_id, server_id, connected_at]).map_err(|error| error.to_string())?;
    database.execute("DELETE FROM recent_sessions WHERE id NOT IN (SELECT id FROM recent_sessions ORDER BY connected_at DESC LIMIT 20)", []).map_err(|error| error.to_string())?;
    drop(database);
    let (sender, receiver) = mpsc::channel();
    let event_app = app.clone();
    let event_session = session_id.clone();
    let connection_host = host.clone();
    let jump_info = if let Some(jump_id) = jump_server_id.as_deref() {
        let database = state.database.lock().map_err(|error| error.to_string())?;
        Some(load_connection_info(&database, jump_id, &request)?.5)
    } else {
        None
    };
    let mut session_info = target_info.clone();
    session_info.jump = jump_info.clone().map(Box::new);
    let output_buffers = state.session_output_buffers.clone();
    std::thread::spawn(move || {
        let emit_state = |status: &str, message: Option<&str>| {
            let phase = (status == "ready").then_some("attach");
            let _ = event_app.emit("sessions:state", serde_json::json!({"sessionId": event_session, "status": status, "phase": phase, "message": message}));
            if status == "error" {
                let _ = event_app.emit("sessions:error", serde_json::json!({"sessionId": event_session, "message": message.unwrap_or("session error"), "recoverable": true}));
            }
        };
        let tcp = if let Some(jump) = jump_info.as_ref() {
            match connect_through_jump(jump, connection_host.as_str(), port, &event_session) {
                Ok(tcp) => tcp,
                Err(error) => { emit_state("error", Some(&error)); return; }
            }
        } else {
            match TcpStream::connect((connection_host.as_str(), port)) { Ok(tcp) => tcp, Err(error) => { emit_state("error", Some(&error.to_string())); return; } }
        };
        let mut session = match ssh2::Session::new() { Ok(session) => session, Err(error) => { emit_state("error", Some(&error.to_string())); return; } };
        session.set_timeout(30_000);
        session.set_tcp_stream(tcp);
        let authenticated = session.handshake().and_then(|_| {
            let actual_fingerprint = host_fingerprint(&session).map_err(|_| ssh2::Error::new(ssh2::ErrorCode::Session(-1), "SSH host key unavailable"))?;
            if expected_fingerprint.as_deref() != Some(actual_fingerprint.as_str()) {
                let request_id = format!("trust-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
                let (trust_tx, trust_rx) = mpsc::channel();
                if let Ok(mut pending) = pending_trust.lock() { pending.insert(request_id.clone(), trust_tx); }
                let _ = event_app.emit("system:hostTrustRequest", serde_json::json!({"requestId": request_id, "kind": if expected_fingerprint.is_some() { "changed" } else { "unknown" }, "serverName": trust_server_name, "host": connection_host, "port": port, "fingerprint": actual_fingerprint, "knownFingerprint": expected_fingerprint}));
                let trusted = trust_rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap_or(false);
                if let Ok(mut pending) = pending_trust.lock() { pending.remove(&request_id); }
                if !trusted { return Err(ssh2::Error::new(ssh2::ErrorCode::Session(-1), "SSH host key is not trusted")); }
                let data_dir = event_app.path().app_data_dir().map_err(|_| ssh2::Error::new(ssh2::ErrorCode::Session(-1), "application data directory unavailable"))?;
                let database = Connection::open(data_dir.join("russh.db")).map_err(|_| ssh2::Error::new(ssh2::ErrorCode::Session(-1), "known host database unavailable"))?;
                database.execute("INSERT OR REPLACE INTO known_hosts (host, port, algorithm, fingerprint, verified_at) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![connection_host, port, "sha256", actual_fingerprint, utc_timestamp()]).map_err(|_| ssh2::Error::new(ssh2::ErrorCode::Session(-1), "known host write failed"))?;
            }
            if let Some(private_key) = connection_private_key.as_deref() {
                let key_path = std::env::temp_dir().join(format!("russh-key-{}", event_session));
                std::fs::write(&key_path, private_key).map_err(|_| ssh2::Error::new(ssh2::ErrorCode::Session(-1), "failed to write private key"))?;
                let result = session.userauth_pubkey_file(&connection_username, None, &key_path, connection_passphrase.as_deref());
                let _ = std::fs::remove_file(key_path);
                result
            } else if let Some(password) = connection_password.as_deref() {
                session.userauth_password(&connection_username, password)
            } else {
                Err(ssh2::Error::new(ssh2::ErrorCode::Session(-1), "password or private key is required"))
            }
        });
        if let Err(error) = authenticated { emit_state("error", Some(&error.to_string())); return; }
        let mut channel = match session.channel_session() { Ok(channel) => channel, Err(error) => { emit_state("error", Some(&error.to_string())); return; } };
        if let Err(error) = channel.request_pty("xterm", None, Some((80, 24, 0, 0))).and_then(|_| channel.shell()) { emit_state("error", Some(&error.to_string())); return; }
        session.set_blocking(false);
        emit_state("ready", None);
        let mut buffer = [0_u8; 8192];
        let mut pending_input = Vec::new();
        loop {
            while let Ok(command) = receiver.try_recv() {
                match command {
                    SessionCommand::Write(data) => pending_input.extend(data),
                    SessionCommand::Resize(columns, rows) => { let _ = channel.request_pty_size(columns as u32, rows as u32, None, None); }
                    SessionCommand::Close => { let _ = channel.close(); return; }
                }
            }

            if !pending_input.is_empty() {
                match channel.write(&pending_input) {
                    Ok(size) if size > 0 => {
                        pending_input.drain(..size);
                        if let Err(error) = channel.flush() {
                            if error.kind() != std::io::ErrorKind::WouldBlock {
                                emit_state("error", Some(&error.to_string()));
                                return;
                            }
                        }
                    }
                    Ok(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => {
                        emit_state("error", Some(&error.to_string()));
                        return;
                    }
                }
            }

            match channel.read(&mut buffer) {
                Ok(size) if size > 0 => { let data = String::from_utf8_lossy(&buffer[..size]).into_owned(); if let Ok(mut pending) = output_buffers.lock() { pending.entry(event_session.clone()).or_default().push_str(&data); } let _ = event_app.emit("sessions:data", serde_json::json!({"sessionId": event_session, "data": data})); if let Some(cwd) = extract_cwd(&data) { let _ = event_app.emit("sessions:cwdChanged", serde_json::json!({"sessionId": event_session, "cwd": cwd})); } }
                Ok(_) => { let _ = event_app.emit("sessions:exit", serde_json::json!({"sessionId": event_session, "exitCode": 0})); return; }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => { std::thread::sleep(std::time::Duration::from_millis(20)); }
                Err(error) => { emit_state("error", Some(&error.to_string())); return; }
            }
        }
    });
    state.sessions.lock().map_err(|error| error.to_string())?.insert(session_id.clone(), sender);
    state.connection_info.lock().map_err(|error| error.to_string())?.insert(session_id.clone(), session_info);
    Ok(serde_json::to_string(&SessionSummaryRecord { session_id, server_id, server_name: name, host, port, status: "connecting".into(), connected_at, current_path: "~".into() }).map_err(|error| error.to_string())?)
}

#[tauri::command]
#[specta::specta]
fn take_session_output(state: tauri::State<'_, RuntimeState>, session_id: String) -> Result<String, String> {
    Ok(state.session_output_buffers.lock().map_err(|error| error.to_string())?.remove(&session_id).unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
fn write_session(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, data: String) -> Result<(), String> {
    if let Some(info) = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned() {
        let mut buffers = state.command_buffers.lock().map_err(|error| error.to_string())?;
        let buffer = buffers.entry(session_id.clone()).or_default();
        buffer.push_str(&data);
        let complete = buffer.ends_with('\n');
        let mut commands: Vec<String> = buffer.split('\n').map(str::to_owned).collect();
        if complete { buffer.clear(); } else { *buffer = commands.pop().unwrap_or_default(); }
        let database = state.database.lock().map_err(|error| error.to_string())?;
        let capture = database.query_row("SELECT enabled != 0 FROM server_preferences WHERE server_id = ?1", [&info.server_id], |row| row.get::<_, bool>(0)).optional().map_err(|error| error.to_string())?.unwrap_or(true);
        if capture {
            for command in commands.into_iter().filter(|command| !command.trim().is_empty()) {
                let command = normalize_entered_command(&command);
                if command.is_empty() { continue; }
                let id = format!("history-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
                let timestamp = utc_timestamp();
                database.execute("INSERT INTO command_history (id, scope_kind, server_id, command, executed_at, cwd, exit_code, duration_ms) VALUES (?1, 'ssh', ?2, ?3, ?4, NULL, NULL, NULL)", rusqlite::params![id, info.server_id, command]).map_err(|error| error.to_string())?;
                let _ = app.emit("commandHistory:added", serde_json::json!({"timestamp": timestamp, "source": "main", "scope": {"kind": "ssh", "serverId": info.server_id}, "entry": {"id": id, "scopeKind": "ssh", "serverId": info.server_id, "command": command, "executedAt": timestamp, "cwd": null, "exitCode": null, "durationMs": null}}));
            }
        }
    }
    let sessions = state.sessions.lock().map_err(|error| error.to_string())?;
    sessions.get(&session_id).ok_or("session not found")?.send(SessionCommand::Write(data.into_bytes())).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn resize_session(state: tauri::State<'_, RuntimeState>, session_id: String, columns: u16, rows: u16) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|error| error.to_string())?;
    sessions.get(&session_id).ok_or("session not found")?.send(SessionCommand::Resize(columns, rows)).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn disconnect_session(state: tauri::State<'_, RuntimeState>, session_id: String) -> Result<(), String> {
    if let Some(sender) = state.sessions.lock().map_err(|error| error.to_string())?.remove(&session_id) { let _ = sender.send(SessionCommand::Close); }
    state.connection_info.lock().map_err(|error| error.to_string())?.remove(&session_id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn reconnect_session(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String) -> Result<String, String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    if let Some(sender) = state.sessions.lock().map_err(|error| error.to_string())?.remove(&session_id) { let _ = sender.send(SessionCommand::Close); }
    let request = serde_json::json!({"sessionId": session_id, "serverId": info.server_id, "secrets": {info.server_id.clone(): {"password": info.password, "privateKey": info.private_key, "passphrase": info.passphrase}}});
    connect_session(app, state, request.to_string())
}

#[tauri::command]
#[specta::specta]
fn get_session_resource_snapshot(session_id: String) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({
        "sessionId": session_id,
        "sampledAt": utc_timestamp(),
        "platform": "unknown",
        "latency": { "rttMs": null },
        "cpu": null,
        "memory": null,
        "network": null,
        "disk": null
    })).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn list_sftp(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, remote_path: String) -> Result<String, String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let sftp = open_sftp(&info)?;
    let resolved_path = resolve_sftp_path(&sftp, &remote_path)?;
    let entries = sftp.readdir(std::path::Path::new(&resolved_path)).map_err(|error| {
        format!("SFTP list {} failed: {error}", resolved_path)
    })?;
    let mapped: Vec<serde_json::Value> = entries.into_iter().map(|(path, stat)| {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default().to_owned();
        let kind = if stat.is_dir() { "directory" } else { "file" };
        serde_json::json!({"path": path.to_string_lossy(), "name": name, "kind": kind, "size": stat.size.unwrap_or(0), "modifiedAt": stat.mtime.map(|value| value.to_string()), "permissions": null})
    }).collect();
    let _ = app.emit("sftp:refresh", serde_json::json!({"sessionId": session_id, "remotePath": resolved_path}));
    serde_json::to_string(&serde_json::json!({"path": resolved_path, "entries": mapped})).map_err(|error| error.to_string())
}

fn resolve_sftp_path(sftp: &ssh2::Sftp, path: &str) -> Result<String, String> {
    if path == "~" || path.starts_with("~/") {
        let home = sftp.realpath(std::path::Path::new(".")).map_err(|error| error.to_string())?;
        if path == "~" {
            return Ok(home.to_string_lossy().into_owned());
        }
        return Ok(home.join(path.trim_start_matches("~/")).to_string_lossy().into_owned());
    }
    Ok(path.to_owned())
}

fn open_sftp(info: &SessionConnectionInfo) -> Result<ssh2::Sftp, String> {
    let tcp = if let Some(jump) = info.jump.as_deref() {
        connect_through_jump(jump, info.host.as_str(), info.port, &format!("{}-sftp", info.server_id))
            .map_err(|error| format!("SFTP via jump {}:{} to {}:{} failed: {error}", jump.host, jump.port, info.host, info.port))?
    } else {
        TcpStream::connect((info.host.as_str(), info.port)).map_err(|error| {
            format!("SFTP target {}:{} connection failed: {error}", info.host, info.port)
        })?
    };
    let mut session = ssh2::Session::new().map_err(|error| error.to_string())?;
    // Keep a stalled SFTP packet from blocking a transfer forever. The relay
    // is non-blocking, so libssh2 must also have a finite I/O deadline.
    session.set_timeout(10_000);
    session.set_tcp_stream(tcp);
    session.handshake().map_err(|error| {
        format!("SFTP target {}:{} SSH handshake failed: {error}", info.host, info.port)
    })?;
    authenticate_session(&mut session, info, "sftp").map_err(|error| {
        format!("SFTP target {}:{} authentication failed: {error}", info.host, info.port)
    })?;
    session.sftp().map_err(|error| {
        format!("SFTP target {}:{} subsystem failed: {error}", info.host, info.port)
    })
}

#[tauri::command]
#[specta::specta]
fn open_sftp_read_stream(state: tauri::State<'_, RuntimeState>, session_id: String, remote_path: String) -> Result<String, String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let sftp = open_sftp(&info)?;
    let stat = sftp.stat(std::path::Path::new(&remote_path)).map_err(|error| error.to_string())?;
    let stream_id = format!("sftp-read-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let file_name = std::path::Path::new(&remote_path).file_name().and_then(|value| value.to_str()).unwrap_or_default();
    state.sftp_streams.lock().map_err(|error| error.to_string())?.insert(stream_id.clone(), SftpStream::Read { info, session_id: session_id.clone(), remote_path: remote_path.clone() });
    serde_json::to_string(&serde_json::json!({"streamId": stream_id, "sessionId": session_id, "remotePath": remote_path, "fileName": file_name, "total": stat.size.unwrap_or(0), "encoding": "utf8"})).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn start_sftp_read_stream(app: AppHandle, state: tauri::State<'_, RuntimeState>, stream_id: String) -> Result<(), String> {
    let stream = state.sftp_streams.lock().map_err(|error| error.to_string())?.remove(&stream_id).ok_or("stream not found")?;
    let SftpStream::Read { info, session_id, remote_path } = stream else { return Err("stream direction mismatch".into()) };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let sftp = open_sftp(&info)?;
            let mut file = sftp.open(std::path::Path::new(&remote_path)).map_err(|error| error.to_string())?;
            let mut buffer = [0_u8; 32 * 1024];
            let mut transferred = 0_u64;
            loop {
                let size = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if size == 0 { break; }
                transferred += size as u64;
                let chunk = String::from_utf8_lossy(&buffer[..size]).into_owned();
                let _ = app.emit("sftp:fileChunk", serde_json::json!({"streamId": stream_id, "sessionId": session_id, "remotePath": remote_path, "chunk": chunk, "transferred": transferred, "total": 0}));
            }
            let _ = app.emit("sftp:fileStreamState", serde_json::json!({"streamId": stream_id, "sessionId": session_id, "remotePath": remote_path, "direction": "download", "status": "completed", "transferred": transferred, "total": transferred, "encoding": "utf8"}));
            Ok(())
        })();
        if let Err(error) = result { let _ = app.emit("sftp:fileStreamState", serde_json::json!({"streamId": stream_id, "sessionId": session_id, "remotePath": remote_path, "direction": "download", "status": "error", "transferred": 0, "total": 0, "error": error})); }
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn open_sftp_write_stream(state: tauri::State<'_, RuntimeState>, session_id: String, remote_path: String, _encoding: String) -> Result<String, String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let stream_id = format!("sftp-write-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    state.sftp_streams.lock().map_err(|error| error.to_string())?.insert(stream_id.clone(), SftpStream::Write { info, session_id: session_id.clone(), remote_path: remote_path.clone(), buffer: String::new() });
    serde_json::to_string(&serde_json::json!({"streamId": stream_id, "sessionId": session_id, "remotePath": remote_path})).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn write_sftp_chunk(state: tauri::State<'_, RuntimeState>, stream_id: String, chunk: String) -> Result<(), String> {
    let mut streams = state.sftp_streams.lock().map_err(|error| error.to_string())?;
    let SftpStream::Write { buffer, .. } = streams.get_mut(&stream_id).ok_or("stream not found")? else { return Err("stream direction mismatch".into()) };
    buffer.push_str(&chunk);
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn close_sftp_write_stream(state: tauri::State<'_, RuntimeState>, stream_id: String) -> Result<(), String> {
    let stream = state.sftp_streams.lock().map_err(|error| error.to_string())?.remove(&stream_id).ok_or("stream not found")?;
    let SftpStream::Write { info, session_id, remote_path, buffer } = stream else { return Err("stream direction mismatch".into()) };
    let sftp = open_sftp(&info)?;
    let mut file = sftp.create(std::path::Path::new(&remote_path)).map_err(|error| error.to_string())?;
    file.write_all(buffer.as_bytes()).map_err(|error| error.to_string())?;
    let _ = (session_id, stream_id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn cancel_sftp_stream(state: tauri::State<'_, RuntimeState>, stream_id: String) -> Result<(), String> {
    state.sftp_streams.lock().map_err(|error| error.to_string())?.remove(&stream_id);
    Ok(())
}

fn session_sftp(state: &tauri::State<'_, RuntimeState>, session_id: &str) -> Result<ssh2::Sftp, String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(session_id).cloned().ok_or("session not found")?;
    open_sftp(&info)
}

#[tauri::command]
#[specta::specta]
fn create_sftp_file(state: tauri::State<'_, RuntimeState>, session_id: String, path: String, name: String) -> Result<(), String> {
    let sftp = session_sftp(&state, &session_id)?;
    let target = std::path::Path::new(&path).join(name);
    sftp.create(&target).map(|_| ()).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn mkdir_sftp(state: tauri::State<'_, RuntimeState>, session_id: String, path: String, name: String) -> Result<(), String> {
    let sftp = session_sftp(&state, &session_id)?;
    sftp.mkdir(&std::path::Path::new(&path).join(name), 0o755).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn rename_sftp(state: tauri::State<'_, RuntimeState>, session_id: String, path: String, new_name: String) -> Result<(), String> {
    let sftp = session_sftp(&state, &session_id)?;
    let source = std::path::Path::new(&path);
    let target = source.parent().unwrap_or_else(|| std::path::Path::new("/")).join(new_name);
    sftp.rename(source, &target, None).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn move_sftp(state: tauri::State<'_, RuntimeState>, session_id: String, source_path: String, destination_dir_path: String) -> Result<(), String> {
    let sftp = session_sftp(&state, &session_id)?;
    let source = std::path::Path::new(&source_path);
    let name = source.file_name().ok_or("source has no file name")?;
    sftp.rename(source, &std::path::Path::new(&destination_dir_path).join(name), None).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn remove_sftp(state: tauri::State<'_, RuntimeState>, session_id: String, path: String) -> Result<(), String> {
    let sftp = session_sftp(&state, &session_id)?;
    let stat = sftp.stat(std::path::Path::new(&path)).map_err(|error| error.to_string())?;
    if stat.is_dir() { sftp.rmdir(std::path::Path::new(&path)) } else { sftp.unlink(std::path::Path::new(&path)) }.map_err(|error| error.to_string())
}

fn upload_local_entry(sftp: &ssh2::Sftp, local: &std::path::Path, remote: &str, cancelled: &AtomicBool, transferred: &mut u64, progress: &dyn Fn(u64)) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) { return Err("transfer cancelled".into()); }
    let metadata = std::fs::metadata(local).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        let _ = sftp.mkdir(std::path::Path::new(remote), 0o755);
        for entry in std::fs::read_dir(local).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let child_remote = format!("{}/{}", remote.trim_end_matches('/'), entry.file_name().to_string_lossy());
            upload_local_entry(sftp, &entry.path(), &child_remote, cancelled, transferred, progress)?;
        }
    } else if metadata.is_file() {
        let mut remote_file = sftp.create(std::path::Path::new(remote)).map_err(|error| error.to_string())?;
        let mut local_file = std::fs::File::open(local).map_err(|error| error.to_string())?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if cancelled.load(Ordering::Relaxed) { return Err("transfer cancelled".into()); }
            let size = local_file.read(&mut buffer).map_err(|error| error.to_string())?;
            if size == 0 { break; }
            remote_file.write_all(&buffer[..size]).map_err(|error| error.to_string())?;
            *transferred += size as u64;
            progress(*transferred);
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn upload_sftp_paths(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, target_path: String, local_paths: Vec<String>) -> Result<(), String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let sftp = open_sftp(&info)?;
    let batch_id = format!("transfer-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let cancelled = Arc::new(AtomicBool::new(false));
    state.transfer_cancellations.lock().map_err(|error| error.to_string())?.insert(batch_id.clone(), cancelled.clone());
    for local_path in local_paths {
        let local = std::path::PathBuf::from(&local_path);
        let name = local.file_name().ok_or("local path has no file name")?.to_string_lossy();
        let remote = format!("{}/{}", target_path.trim_end_matches('/'), name);
        let _ = app.emit("sftp:transferProgress", serde_json::json!({"batchId": batch_id, "sessionId": session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": 0, "total": metadata_len(&local), "status": "running"}));
        let mut transferred = 0_u64;
        let progress_app = app.clone();
        let progress_batch = batch_id.clone();
        let progress_session = session_id.clone();
        let progress_file_name = name.to_string();
        let progress_local = local_path.clone();
        let progress_remote = remote.clone();
        let total = metadata_len(&local);
        let progress = move |bytes| { let _ = progress_app.emit("sftp:transferProgress", serde_json::json!({"batchId": progress_batch, "sessionId": progress_session, "direction": "upload", "fileName": progress_file_name, "localPath": progress_local, "remotePath": progress_remote, "transferred": bytes, "total": total, "status": "running"})); };
        if let Err(error) = upload_local_entry(&sftp, &local, &remote, &cancelled, &mut transferred, &progress) {
            let status = if cancelled.load(Ordering::Relaxed) { "cancelled" } else { "error" };
            let _ = app.emit("sftp:transferProgress", serde_json::json!({"batchId": batch_id, "sessionId": session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": 0, "total": metadata_len(&local), "status": status, "error": error}));
            state.transfer_cancellations.lock().ok().map(|mut values| values.remove(&batch_id));
            return Err(error);
        }
        let _ = app.emit("sftp:transferProgress", serde_json::json!({"batchId": batch_id, "sessionId": session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": metadata_len(&local), "total": metadata_len(&local), "status": "completed"}));
    }
    state.transfer_cancellations.lock().ok().map(|mut values| values.remove(&batch_id));
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn cancel_sftp_transfer(state: tauri::State<'_, RuntimeState>, batch_id: String) -> Result<(), String> {
    if let Some(token) = state.transfer_cancellations.lock().map_err(|error| error.to_string())?.get(&batch_id) { token.store(true, Ordering::Relaxed); }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn cancel_all_sftp_transfers(state: tauri::State<'_, RuntimeState>) -> Result<(), String> {
    for token in state.transfer_cancellations.lock().map_err(|error| error.to_string())?.values() { token.store(true, Ordering::Relaxed); }
    Ok(())
}

fn metadata_len(path: &std::path::Path) -> u64 { std::fs::metadata(path).map(|value| value.len()).unwrap_or(0) }

fn format_transfer_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{} {}", bytes, UNITS[unit]) } else { format!("{value:.2} {}", UNITS[unit]) }
}

#[tauri::command]
#[specta::specta]
fn download_sftp_file(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, remote_path: String) -> Result<(), String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let file_name = std::path::Path::new(&remote_path).file_name().and_then(|value| value.to_str()).unwrap_or("download").to_owned();
    let dialog_app = app.clone();
    app.dialog().file().set_file_name(&file_name).save_file(move |path| {
        let Some(path) = path.map(|value| value.to_string()) else { return; };
        let download_app = dialog_app.clone();
        let download_session_id = session_id.clone();
        let download_remote_path = remote_path.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<u64, String> {
                let sftp = open_sftp(&info)?;
                let mut remote = sftp.open(std::path::Path::new(&download_remote_path)).map_err(|error| {
                    format!("SFTP open {}:{} failed for {}: {error}", info.host, info.port, download_remote_path)
                })?;
                let total = remote.stat().map_err(|error| {
                    format!("SFTP stat {}:{} failed for {}: {error}", info.host, info.port, download_remote_path)
                })?.size.unwrap_or(0);
                let local_size = std::fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
                let mut resume_download = local_size > 0 && local_size < total;
                if local_size > 0 {
                    let (question, title) = if local_size < total {
                        (
                            format!(
                                "本地文件已有 {}，远端文件大小为 {}。是否从本地位置继续断点续传？",
                                format_transfer_size(local_size),
                                format_transfer_size(total)
                            ),
                            "继续断点续传",
                        )
                    } else {
                        (
                            format!(
                                "本地文件大小为 {}，远端文件大小为 {}。是否覆盖本地文件并重新下载？",
                                format_transfer_size(local_size),
                                format_transfer_size(total)
                            ),
                            "覆盖本地文件",
                        )
                    };
                    let (answer_tx, answer_rx) = mpsc::channel();
                    download_app
                        .dialog()
                        .message(question)
                        .title(title)
                        .buttons(MessageDialogButtons::YesNo)
                        .show(move |answer| {
                            let _ = answer_tx.send(answer);
                        });
                    let accepted = answer_rx.recv().unwrap_or(false);
                    if local_size < total {
                        // For a partial local file, No means overwrite and restart
                        // from byte zero; it is not a cancellation.
                        resume_download = accepted;
                    } else if !accepted {
                        return Err("用户取消下载".into());
                    }
                }
                // The local file length is the only safe resume point: every byte before it
                // has already been persisted. If the local file is longer than the remote
                // file, it cannot be a valid prefix and must be restarted from byte zero.
                let resume_offset = if resume_download { local_size } else { 0 };
                if resume_offset > 0 {
                    remote.seek(SeekFrom::Start(resume_offset)).map_err(|error| {
                        format!("SFTP resume seek {}:{} failed at {} bytes: {error}", info.host, info.port, resume_offset)
                    })?;
                }
                let mut local = std::fs::OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(false)
                    .open(&path)
                    .map_err(|error| error.to_string())?;
                // Remove any stale tail before appending the resumed blocks. This also
                // guarantees that a same-size/empty file is not accidentally extended.
                local.set_len(resume_offset).map_err(|error| error.to_string())?;
                local.seek(SeekFrom::Start(resume_offset)).map_err(|error| error.to_string())?;
                let _ = download_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": download_session_id, "direction": "download", "fileName": file_name, "localPath": path, "remotePath": download_remote_path, "transferred": resume_offset, "total": total, "status": "running"}));
                let mut transferred = resume_offset;
                let mut last_reported = std::time::Instant::now();
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let size = remote.read(&mut buffer).map_err(|error| {
                        format!("SFTP read {}:{} failed for {} after {} bytes: {error}", info.host, info.port, download_remote_path, transferred)
                    })?;
                    if size == 0 { break; }
                    local.write_all(&buffer[..size]).map_err(|error| error.to_string())?;
                    transferred += size as u64;
                    // Progress is UI telemetry, not part of the data path.
                    // Emitting once per SFTP read can flood the webview when
                    // libssh2 returns small packets over a jump connection.
                    if last_reported.elapsed() >= std::time::Duration::from_millis(100)
                        || (total > 0 && transferred >= total)
                    {
                        let _ = download_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": download_session_id, "direction": "download", "fileName": file_name, "localPath": path, "remotePath": download_remote_path, "transferred": transferred, "total": total, "status": "running"}));
                        last_reported = std::time::Instant::now();
                    }
                }
                Ok(transferred)
            })();
            match result {
                Ok(bytes) => { let _ = download_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": download_session_id, "direction": "download", "fileName": file_name, "localPath": path, "remotePath": download_remote_path, "transferred": bytes, "total": bytes, "status": "completed"})); }
                Err(error) => { let _ = download_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": download_session_id, "direction": "download", "fileName": file_name, "localPath": path, "remotePath": download_remote_path, "transferred": 0, "total": 0, "status": "error", "error": error})); }
            }
        });
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn upload_sftp_files(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, target_path: String) -> Result<(), String> {
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let dialog_app = app.clone();
    app.dialog().file().add_filter("All files", &["*"]).pick_files(move |paths| {
        let Some(local_paths) = paths.map(|values| values.into_iter().map(|value| value.to_string()).collect::<Vec<_>>()) else { return; };
        let upload_session_id = session_id.clone();
        let upload_target_path = target_path.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let sftp = open_sftp(&info)?;
                for local_path in local_paths {
                    let local = std::path::PathBuf::from(&local_path);
                    let name = local.file_name().ok_or("local path has no file name")?.to_string_lossy().into_owned();
                    let remote = format!("{}/{}", upload_target_path.trim_end_matches('/'), name);
                    let total = metadata_len(&local);
                    let _ = dialog_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": upload_session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": 0, "total": total, "status": "running"}));
                    let mut transferred = 0_u64;
                    upload_local_entry(&sftp, &local, &remote, &AtomicBool::new(false), &mut transferred, &|bytes| {
                        let _ = dialog_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": upload_session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": bytes, "total": total, "status": "running"}));
                    })?;
                    let _ = dialog_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": upload_session_id, "direction": "upload", "fileName": name, "localPath": local_path, "remotePath": remote, "transferred": transferred, "total": total, "status": "completed"}));
                }
                Ok(())
            })();
            if let Err(error) = result { let _ = dialog_app.emit("sftp:transferProgress", serde_json::json!({"sessionId": upload_session_id, "direction": "upload", "fileName": "", "remotePath": upload_target_path, "transferred": 0, "total": 0, "status": "error", "error": error})); }
        });
    });
    Ok(())
}

fn port_forward_json(rule: &PortForwardRecord, status: &str) -> serde_json::Value {
    serde_json::json!({"id": rule.id, "sessionId": rule.session_id, "kind": rule.kind, "bindHost": rule.bind_host, "bindPort": rule.bind_port, "targetHost": rule.target_host, "targetPort": rule.target_port, "enabled": rule.enabled, "status": status, "createdAt": "", "updatedAt": ""})
}

#[tauri::command]
#[specta::specta]
fn list_port_forwards(state: tauri::State<'_, RuntimeState>, session_id: String) -> Result<String, String> {
    let rules = state.port_forward_rules.lock().map_err(|error| error.to_string())?;
    let values: Vec<_> = rules.values().filter(|rule| rule.session_id == session_id).map(|rule| port_forward_json(rule, if rule.enabled { "active" } else { "stopped" })).collect();
    serde_json::to_string(&values).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn create_port_forward(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, input: String) -> Result<String, String> {
    let event_session_id = session_id.clone();
    let value: serde_json::Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    let rule = PortForwardRecord { id: format!("forward-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()), session_id, kind: value.get("kind").and_then(|value| value.as_str()).unwrap_or("local").into(), bind_host: value.get("bindHost").and_then(|value| value.as_str()).unwrap_or("127.0.0.1").into(), bind_port: value.get("bindPort").and_then(|value| value.as_u64()).unwrap_or(0) as u16, target_host: value.get("targetHost").and_then(|value| value.as_str()).ok_or("targetHost is required")?.into(), target_port: value.get("targetPort").and_then(|value| value.as_u64()).unwrap_or(22) as u16, enabled: false };
    let json = port_forward_json(&rule, "stopped");
    state.port_forward_rules.lock().map_err(|error| error.to_string())?.insert(rule.id.clone(), rule);
    let _ = app.emit("portForwards:state", serde_json::json!({"sessionId": event_session_id, "rule": json}));
    serde_json::to_string(&json).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn start_port_forward(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, rule_id: String) -> Result<String, String> {
    let rule = state.port_forward_rules.lock().map_err(|error| error.to_string())?.get(&rule_id).cloned().ok_or("port forward not found")?;
    if rule.session_id != session_id { return Err("port forward session mismatch".into()); }
    let info = state.connection_info.lock().map_err(|error| error.to_string())?.get(&session_id).cloned().ok_or("session not found")?;
    let listener = TcpListener::bind((rule.bind_host.as_str(), rule.bind_port)).map_err(|error| error.to_string())?;
    listener.set_nonblocking(true).map_err(|error| error.to_string())?;
    let (stop_tx, stop_rx) = mpsc::channel();
    state.port_forward_stops.lock().map_err(|error| error.to_string())?.insert(rule_id.clone(), stop_tx);
    let thread_rule = rule.clone();
    std::thread::spawn(move || loop {
        if stop_rx.try_recv().is_ok() { break; }
        match listener.accept() {
            Ok((mut local, _)) => {
                let connection = info.clone();
                let target_host = thread_rule.target_host.clone();
                let target_port = thread_rule.target_port;
                std::thread::spawn(move || {
                    let result = (|| -> Result<(), String> {
                        let tcp = TcpStream::connect((connection.host.as_str(), connection.port)).map_err(|error| error.to_string())?;
                        let mut ssh = ssh2::Session::new().map_err(|error| error.to_string())?;
                        ssh.set_tcp_stream(tcp); ssh.handshake().map_err(|error| error.to_string())?; authenticate_session(&mut ssh, &connection, "forward")?;
                        let remote = ssh.channel_direct_tcpip(&target_host, target_port, None).map_err(|error| error.to_string())?;
                        let mut remote_to_local = remote.stream(0);
                        let mut local_to_remote = remote.stream(0);
                        let _ = std::io::copy(&mut local, &mut local_to_remote);
                        let _ = std::io::copy(&mut remote_to_local, &mut local);
                        Ok(())
                    })();
                    let _ = result;
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(_) => break,
        }
    });
    let json = port_forward_json(&rule, "active");
    let _ = app.emit("portForwards:state", serde_json::json!({"sessionId": session_id, "rule": json}));
    Ok(serde_json::to_string(&json).map_err(|error| error.to_string())?)
}

#[tauri::command]
#[specta::specta]
fn stop_port_forward(app: AppHandle, state: tauri::State<'_, RuntimeState>, session_id: String, rule_id: String) -> Result<(), String> {
    if let Some(sender) = state.port_forward_stops.lock().map_err(|error| error.to_string())?.remove(&rule_id) { let _ = sender.send(()); }
    if let Some(rule) = state.port_forward_rules.lock().map_err(|error| error.to_string())?.get_mut(&rule_id) { rule.enabled = false; let json = port_forward_json(rule, "stopped"); let _ = app.emit("portForwards:state", serde_json::json!({"sessionId": session_id, "rule": json})); }
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn remove_port_forward(state: tauri::State<'_, RuntimeState>, _session_id: String, rule_id: String) -> Result<(), String> {
    if let Some(sender) = state.port_forward_stops.lock().map_err(|error| error.to_string())?.remove(&rule_id) { let _ = sender.send(()); }
    state.port_forward_rules.lock().map_err(|error| error.to_string())?.remove(&rule_id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn window_minimize(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn window_toggle_maximize(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())
    } else {
        window.maximize().map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
fn window_close(window: WebviewWindow) -> Result<(), String> {
    window.close().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn window_is_maximized(window: WebviewWindow) -> Result<bool, String> {
    window.is_maximized().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn get_path_for_file(path: String) -> Result<String, String> {
    std::fs::canonicalize(path)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn get_shell_integration_script() -> String {
    r#"__wsh_emit() { printf '\033]%s\033\\' "$1"; }; __wsh_cwd() { __wsh_emit "133;P;Cwd=$PWD"; __wsh_emit "7;file://$PWD"; }; if [ -n "$BASH_VERSION" ]; then __wsh_post() { __wsh_ec=$?; __wsh_cwd; __wsh_emit "133;D;$__wsh_ec"; __wsh_emit "133;A"; }; PROMPT_COMMAND="__wsh_post${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; elif [ -n "$ZSH_VERSION" ]; then __wsh_post() { __wsh_cwd; __wsh_emit "133;A"; }; autoload -Uz add-zsh-hook >/dev/null 2>&1; add-zsh-hook precmd __wsh_post >/dev/null 2>&1; fi"#.to_owned()
}

#[tauri::command]
#[specta::specta]
fn pick_private_key(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = mpsc::channel();
    app.dialog().file().add_filter("Private key", &["pem", "key", "ppk"]).pick_file(move |path| { let _ = sender.send(path.map(|value| value.to_string())); });
    receiver.recv_timeout(std::time::Duration::from_secs(300)).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
fn pick_server_icon(app: AppHandle) -> Result<Option<ServerIconRecord>, String> {
    let (sender, receiver) = mpsc::channel();
    app.dialog().file().add_filter("Images", &["png", "jpg", "jpeg", "webp", "svg"]).pick_file(move |path| { let _ = sender.send(path.map(|value| value.to_string())); });
    let Some(path) = receiver.recv_timeout(std::time::Duration::from_secs(300)).map_err(|error| error.to_string())? else { return Ok(None); };
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let mime = match std::path::Path::new(&path).extension().and_then(|value| value.to_str()).unwrap_or("png").to_ascii_lowercase().as_str() { "jpg" | "jpeg" => "image/jpeg", "webp" => "image/webp", "svg" => "image/svg+xml", _ => "image/png" };
    Ok(Some(ServerIconRecord { mime_type: mime.into(), data: bytes }))
}

#[tauri::command]
#[specta::specta]
fn relaunch_application(app: AppHandle) -> Result<(), String> {
    app.restart();
}

pub fn run() {
    let builder = Builder::<tauri::Wry>::new().commands(collect_commands![
        get_app_info,
        get_system_capabilities,
        list_themes,
        import_theme_archive,
        delete_theme_plugin,
        get_update_state,
        check_for_updates,
        get_log_state,
        list_logs,
        clear_logs,
        update_log_path,
        write_log,
        get_settings,
        update_settings,
        test_webdav_connection,
        list_webdav_backups,
        backup_webdav_now,
        delete_webdav_backup,
        restore_webdav_backup,
        list_groups,
        create_group,
        delete_group,
        update_group,
        list_tags,
        create_tag,
        delete_tag,
        update_tag,
        list_servers,
        list_recent_servers,
        clear_recent_servers,
        create_server,
        delete_server,
        update_server,
        get_server_secrets,
        export_server_config,
        import_server_config,
        toggle_server_favorite,
        list_credentials,
        get_credential_secret,
        create_credential,
        delete_credential,
        update_credential,
        database_health,
        list_known_hosts,
        remove_known_host,
        respond_host_trust,
        create_local_terminal,
        write_local_terminal,
        resize_local_terminal,
        close_local_terminal,
        connect_session,
        write_session,
        resize_session,
        disconnect_session,
        reconnect_session,
        get_session_resource_snapshot,
        take_session_output,
        list_sftp,
        open_sftp_read_stream,
        start_sftp_read_stream,
        open_sftp_write_stream,
        write_sftp_chunk,
        close_sftp_write_stream,
        cancel_sftp_stream,
        create_sftp_file,
        mkdir_sftp,
        rename_sftp,
        move_sftp,
        remove_sftp,
        upload_sftp_paths,
        cancel_sftp_transfer,
        cancel_all_sftp_transfers,
        download_sftp_file,
        upload_sftp_files,
        list_port_forwards,
        create_port_forward,
        start_port_forward,
        stop_port_forward,
        remove_port_forward,
        list_command_history,
        search_command_history,
        clear_command_history,
        clear_all_command_history,
        delete_command_history,
        set_server_capture,
        list_custom_commands,
        create_custom_command,
        update_custom_command,
        delete_custom_command,
        list_sftp_bookmarks,
        add_sftp_bookmark,
        delete_sftp_bookmark,
        delete_sftp_bookmark_by_path,
        window_minimize,
        window_toggle_maximize,
        window_close,
        window_is_maximized,
        get_path_for_file,
        get_shell_integration_script,
        pick_private_key,
        pick_server_icon,
        relaunch_application,
    ]);

    #[cfg(debug_assertions)]
    builder
        .export(Typescript::default(), concat!(env!("CARGO_MANIFEST_DIR"), "/../src/shared/tauri-bindings.ts"))
        .expect("export Tauri bindings");

    tauri::Builder::default()
        .plugin(CompatibilityPlugin)
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = RuntimeState::open(&app.handle())?;
            app.manage(state);
            let menu = MenuBuilder::new(app)
                .text("openSettings", "Settings")
                .text("openNewConnection", "New Connection")
                .text("openLocalTerminal", "Local Terminal")
                .text("quit", "Quit")
                .build()?;
            let decoder = png::Decoder::new(Cursor::new(include_bytes!("../../build/icon.png")));
            let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
            let mut pixels = vec![0; reader.output_buffer_size().ok_or("invalid tray icon")?];
            let info = reader.next_frame(&mut pixels).map_err(|error| error.to_string())?;
            let tray_icon = tauri::image::Image::new_owned(pixels[..info.buffer_size()].to_vec(), info.width, info.height);
            TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "quit" {
                        app.exit(0);
                    } else {
                        let _ = app.emit("system:menuAction", event.id().as_ref());
                    }
                })
                .on_tray_icon_event(|_tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        // The renderer remains the owner of focus and window state.
                    }
                })
                .build(app)?;
            let window = app.get_webview_window("main").ok_or("main window is missing")?;
            let event_app = app.handle().clone();
            window.on_window_event(move |event| {
                match event {
                    tauri::WindowEvent::Resized(_) => {
                        if let Some(is_maximized) = event_app.get_webview_window("main").and_then(|window| window.is_maximized().ok()) {
                            let _ = event_app.emit("system:windowState", WindowState { is_maximized });
                        }
                    }
                    _ => {}
                }
            });
            window.show()?;
            Ok(())
        })
        .invoke_handler(builder.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running RuSSH");
}

#[cfg(test)]
mod tests {
    use super::{extract_cwd, normalize_entered_command, safe_theme_component};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn extracts_osc7_cwd() {
        assert_eq!(extract_cwd("\u{1b}]7;file://host/home/user\u{7}"), Some("/home/user".into()));
    }

    #[test]
    fn extracts_osc133_cwd() {
        assert_eq!(extract_cwd("\u{1b}]133;P;Cwd=/tmp/project\u{1b}\\"), Some("/tmp/project".into()));
    }

    #[test]
    fn ignores_output_without_cwd_marker() {
        assert_eq!(extract_cwd("ordinary terminal output"), None);
    }

    #[test]
    fn rejects_unsafe_theme_plugin_components() {
        assert!(safe_theme_component("publisher-theme"));
        assert!(!safe_theme_component("../escape"));
        assert!(!safe_theme_component("theme/name"));
        assert!(!safe_theme_component(""));
    }

    #[test]
    fn transfer_cancellation_token_is_shared() {
        let token = AtomicBool::new(false);
        assert!(!token.load(Ordering::Relaxed));
        token.store(true, Ordering::Relaxed);
        assert!(token.load(Ordering::Relaxed));
    }

    #[test]
    fn normalizes_terminal_command_input() {
        assert_eq!(normalize_entered_command("ecx\u{7f}ho\u{1b}[D\n"), "echo");
    }
}
