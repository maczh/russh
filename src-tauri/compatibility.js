(() => {
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args)
  const unsupported = (path) => () =>
    Promise.reject(new Error(`Tauri migration pending: ${path}`))
  const listen = (event, callback) => {
    let unlisten = null
    let cancelled = false
    const registration = window.__TAURI__.event.listen(event, (payload) => callback(payload.payload))
    void registration.then((handler) => {
      if (cancelled) {
        void handler()
      } else {
        unlisten = handler
      }
    })
    return () => {
      cancelled = true
      if (unlisten) {
        void unlisten()
      }
    }
  }

  const domains = [
    'groups', 'tags', 'credentials', 'servers', 'sessions', 'localTerminals',
    'sftp', 'portForwards', 'commandHistory', 'customCommands', 'sftpBookmarks',
    'settings', 'logs', 'updates', 'themes', 'backup'
  ]
  const api = {}
  for (const domain of domains) {
    api[domain] = new Proxy({}, {
      get: (_target, method) => unsupported(`${String(domain)}.${String(method)}`)
    })
  }

  api.system = {
    getAppInfo: () => invoke('get_app_info'),
    getCapabilities: () => invoke('get_system_capabilities'),
    pickPrivateKey: () => invoke('pick_private_key'),
    pickServerIcon: () => invoke('pick_server_icon'),
    getShellIntegrationScript: () => invoke('get_shell_integration_script'),
    relaunch: () => invoke('relaunch_application'),
    getKnownHosts: () => invoke('list_known_hosts'),
    removeKnownHost: (host, port) => invoke('remove_known_host', { host, port }),
    respondHostTrust: (result) => invoke('respond_host_trust', { result: JSON.stringify(result) }),
    getPathForFile: (file) => {
      if (!file || typeof file !== 'object') return null
      const path = file.path
      return typeof path === 'string' && path.length > 0 ? path : null
    },
    menu: { onAction: (callback) => listen('system:menuAction', callback) },
    onHostTrustRequest: (callback) => listen('system:hostTrustRequest', callback),
    window: {
      minimize: () => invoke('window_minimize'),
      toggleMaximize: () => invoke('window_toggle_maximize'),
      close: () => invoke('window_close'),
      isMaximized: () => invoke('window_is_maximized'),
      onStateChange: (callback) => listen('system:windowState', callback)
    }
  }
  api.themes = {
    list: async () => JSON.parse(await invoke('list_themes')),
    importArchive: () => invoke('import_theme_archive'),
    deletePlugin: (pluginId) => invoke('delete_theme_plugin', { pluginId })
  }
  api.updates = {
    getState: () => invoke('get_update_state'),
    check: () => invoke('check_for_updates'),
    download: unsupported('updates.download'),
    quitAndInstall: unsupported('updates.quitAndInstall'),
    onStateChange: (callback) => listen('updates:state', callback)
  }
  api.logs = {
    clear: () => invoke('clear_logs'),
    getState: async () => JSON.parse(await invoke('get_log_state')),
    list: () => invoke('list_logs'),
    updatePath: async (path) => JSON.parse(await invoke('update_log_path', { path })),
    write: (event) => invoke('write_log', { event: JSON.stringify(event) })
  }
  api.backup = {
    getState: async () => ({ lastBackupAt: null, lastBackupError: null, nextBackupAt: null }),
    list: () => invoke('list_webdav_backups'),
    backupNow: () => invoke('backup_webdav_now'),
    delete: (fileName) => invoke('delete_webdav_backup', { fileName }),
    restore: (fileName) => invoke('restore_webdav_backup', { fileName }),
    testConnection: () => invoke('test_webdav_connection')
  }
  api.settings = {
    get: () => invoke('get_settings'),
    update: (input) => invoke('update_settings', { input: JSON.stringify(input) })
  }
  api.groups = {
    list: () => invoke('list_groups'),
    create: (input) => invoke('create_group', { input: JSON.stringify(input) }),
    update: (id, input) => invoke('update_group', { id, input: JSON.stringify(input) }),
    delete: (id) => invoke('delete_group', { id })
  }
  api.tags = {
    list: () => invoke('list_tags'),
    create: (input) => invoke('create_tag', { input: JSON.stringify(input) }),
    update: (id, input) => invoke('update_tag', { id, input: JSON.stringify(input) }),
    delete: (id) => invoke('delete_tag', { id })
  }
  api.servers = {
    list: async () => JSON.parse(await invoke('list_servers')),
    findById: async (id) => (JSON.parse(await invoke('list_servers'))).find((server) => server.id === id) ?? null,
    getSecrets: async (id) => JSON.parse(await invoke('get_server_secrets', { id })),
    create: async (input) => JSON.parse(await invoke('create_server', { input: JSON.stringify(input) })),
    update: async (id, input) => JSON.parse(await invoke('update_server', { id, input: JSON.stringify(input) })),
    delete: (id) => invoke('delete_server', { id }),
    toggleFavorite: async (id) => JSON.parse(await invoke('toggle_server_favorite', { id })),
    listRecent: () => invoke('list_recent_servers'),
    clearRecent: () => invoke('clear_recent_servers')
  }
  api.credentials = {
    list: () => invoke('list_credentials'),
    getSecret: (id) => invoke('get_credential_secret', { id }),
    create: (input) => invoke('create_credential', { input: JSON.stringify(input) }),
    update: (id, input) => invoke('update_credential', { id, input: JSON.stringify(input) }),
    delete: (id) => invoke('delete_credential', { id })
  }
  api.localTerminals = {
    create: () => invoke('create_local_terminal'),
    close: (terminalId) => invoke('close_local_terminal', { terminalId }),
    write: (terminalId, data) => { void invoke('write_local_terminal', { terminalId, data }) },
    resize: (terminalId, columns, rows) => invoke('resize_local_terminal', { terminalId, columns, rows }),
    onData: (terminalId, callback) => listen('localTerminals:data', (event) => event.terminalId === terminalId && callback(event)),
    onExit: (callback) => listen('localTerminals:exit', callback),
    onStateChange: (callback) => listen('localTerminals:state', callback)
  }
  api.sessions = {
    connect: async (request) => {
      try {
        const summary = JSON.parse(await invoke('connect_session', { input: JSON.stringify(request) }))
        return { ok: true, summary }
      } catch (error) {
        return { ok: false, code: 'connection_failed', message: String(error), serverId: request.serverId }
      }
    },
    disconnect: (sessionId) => invoke('disconnect_session', { sessionId }),
    reconnect: async (sessionId) => JSON.parse(await invoke('reconnect_session', { sessionId })),
    getResourceSnapshot: async (sessionId) => JSON.parse(await invoke('get_session_resource_snapshot', { sessionId })),
    write: (sessionId, data) => { void invoke('write_session', { sessionId, data }) },
    resize: (sessionId, columns, rows) => invoke('resize_session', { sessionId, columns, rows }),
    createDataChannel: async (sessionId) => {
      const channel = new MessageChannel()
      let sequence = 0
      let closed = false
      const unsubscribe = listen('sessions:data', (event) => {
        if (closed || event.sessionId !== sessionId) return
        const payload = new TextEncoder().encode(event.data ?? '')
        const frame = new ArrayBuffer(16 + payload.byteLength)
        const view = new DataView(frame)
        view.setUint32(0, sequence++, false)
        view.setFloat64(4, Date.now(), false)
        view.setUint32(12, payload.byteLength, false)
        new Uint8Array(frame, 16).set(payload)
        try {
          channel.port1.postMessage({ type: 'data', frame }, [frame])
        } catch {
          // The renderer may close the worker port while an event is in flight.
          // Stop the subscription and discard late frames instead of creating an
          // unhandled promise rejection from MessagePort.postMessage().
          closed = true
          unsubscribe()
        }
      })
      channel.port1.addEventListener('message', (event) => {
        if (event.data?.type === 'close') {
          closed = true
          unsubscribe()
          channel.port1.close()
          channel.port2.close()
        }
      })
      channel.port1.start()
      return channel.port2
    },
    onData: (sessionId, callback) => {
      const unsubscribe = listen('sessions:data', (event) => event.sessionId === sessionId && callback(event))
      void invoke('take_session_output', { sessionId }).then((data) => {
        if (data) callback({ sessionId, data })
      })
      return unsubscribe
    },
    onExit: (callback) => listen('sessions:exit', callback),
    onStateChange: (callback) => listen('sessions:state', callback),
    onError: (callback) => listen('sessions:error', callback),
    onCwdChanged: (callback) => listen('sessions:cwdChanged', callback),
    onTerminalBackpressure: (callback) => listen('terminal:backpressure', callback),
    onTerminalDegraded: (callback) => listen('terminal:degraded', callback)
  }
  api.sftp = {
    list: async (sessionId, remotePath) => JSON.parse(await invoke('list_sftp', { sessionId, remotePath })),
    refresh: async (sessionId, remotePath) => JSON.parse(await invoke('list_sftp', { sessionId, remotePath })),
    createFile: (sessionId, path, name) => invoke('create_sftp_file', { sessionId, path, name }),
    mkdir: (sessionId, path, name) => invoke('mkdir_sftp', { sessionId, path, name }),
    rename: (sessionId, path, newName) => invoke('rename_sftp', { sessionId, path, newName }),
    move: (sessionId, sourcePath, destinationDirPath) => invoke('move_sftp', { sessionId, sourcePath, destinationDirPath }),
    remove: (sessionId, path) => invoke('remove_sftp', { sessionId, path }),
    uploadPaths: (sessionId, targetPath, localPaths) => invoke('upload_sftp_paths', { sessionId, targetPath, localPaths }),
    uploadFiles: (sessionId, targetPath) => invoke('upload_sftp_files', { sessionId, targetPath }), downloadFile: (sessionId, remotePath) => invoke('download_sftp_file', { sessionId, remotePath }),
    cancelTransfer: (batchId) => invoke('cancel_sftp_transfer', { batchId }),
    cancelAllTransfers: () => invoke('cancel_all_sftp_transfers'),
    openFileReadStream: async (sessionId, remotePath) => JSON.parse(await invoke('open_sftp_read_stream', { sessionId, remotePath })),
    startFileReadStream: (streamId) => { void invoke('start_sftp_read_stream', { streamId }) },
    openFileWriteStream: async (sessionId, remotePath, encoding) => JSON.parse(await invoke('open_sftp_write_stream', { sessionId, remotePath, encoding })),
    writeFileChunk: (streamId, chunk) => invoke('write_sftp_chunk', { streamId, chunk }),
    closeFileWriteStream: (streamId) => invoke('close_sftp_write_stream', { streamId }),
    cancelFileStream: (streamId) => { void invoke('cancel_sftp_stream', { streamId }) },
    onFileChunk: (callback) => listen('sftp:fileChunk', callback),
    onFileStreamState: (callback) => listen('sftp:fileStreamState', callback),
    onTransferProgress: (callback) => listen('sftp:transferProgress', callback)
  }
  api.portForwards = {
    list: async (sessionId) => JSON.parse(await invoke('list_port_forwards', { sessionId })),
    create: async (sessionId, input) => JSON.parse(await invoke('create_port_forward', { sessionId, input: JSON.stringify(input) })),
    start: async (sessionId, ruleId) => JSON.parse(await invoke('start_port_forward', { sessionId, ruleId })),
    stop: (sessionId, ruleId) => invoke('stop_port_forward', { sessionId, ruleId }),
    remove: (sessionId, ruleId) => invoke('remove_port_forward', { sessionId, ruleId }),
    onStateChange: (callback) => listen('portForwards:state', callback)
  }
  api.commandHistory = {
    list: (input) => invoke('list_command_history', { input: JSON.stringify(input) }),
    search: (input) => invoke('search_command_history', { input: JSON.stringify(input) }),
    clear: (scope) => invoke('clear_command_history', { scope: JSON.stringify(scope) }),
    clearAll: () => invoke('clear_all_command_history'),
    deleteEntry: (id) => invoke('delete_command_history', { id }),
    setServerCapture: (serverId, enabled) => invoke('set_server_capture', { serverId, enabled }),
    onCommandAdded: (callback) => listen('commandHistory:added', callback)
  }
  api.customCommands = {
    list: () => invoke('list_custom_commands'),
    create: (input) => invoke('create_custom_command', { input: JSON.stringify(input) }),
    update: (id, input) => invoke('update_custom_command', { id, input: JSON.stringify(input) }),
    delete: (id) => invoke('delete_custom_command', { id })
  }
  api.sftpBookmarks = {
    list: (serverId) => invoke('list_sftp_bookmarks', { serverId }),
    add: (serverId, path) => invoke('add_sftp_bookmark', { serverId, path }),
    delete: (id) => invoke('delete_sftp_bookmark', { id }),
    deleteByPath: (serverId, path) => invoke('delete_sftp_bookmark_by_path', { serverId, path })
  }

  window.russhApi = api
})()
