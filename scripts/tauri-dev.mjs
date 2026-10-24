import { spawn } from 'node:child_process'
import net from 'node:net'
import { resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const vite = spawn(process.execPath, [resolve(root, 'node_modules/vite/bin/vite.js'), '--config', 'vite.tauri.config.ts'], {
  cwd: root,
  stdio: 'inherit',
  env: process.env
})

const waitForVite = () => new Promise((resolveReady, reject) => {
  const deadline = Date.now() + 30_000
  const probe = () => {
    const socket = net.createConnection({ host: '127.0.0.1', port: 5173 })
    socket.once('connect', () => {
      socket.destroy()
      resolveReady()
    })
    socket.once('error', () => {
      socket.destroy()
      if (Date.now() >= deadline) reject(new Error('Vite dev server did not start on 127.0.0.1:5173'))
      else setTimeout(probe, 100)
    })
  }
  probe()
})

await waitForVite()

const cargo = spawn('cargo', ['run', '--manifest-path', 'src-tauri/Cargo.toml', '--no-default-features'], {
  cwd: root,
  stdio: 'inherit',
  env: process.env
})

let shuttingDown = false
const shutdown = (code = 0) => {
  if (shuttingDown) return
  shuttingDown = true
  vite.kill('SIGTERM')
  cargo.kill('SIGTERM')
  setTimeout(() => process.exit(code), 250)
}

vite.once('error', () => shutdown(1))
cargo.once('error', () => shutdown(1))
vite.once('exit', (code) => {
  if (!shuttingDown && code !== 0) shutdown(code ?? 1)
})
cargo.once('exit', (code) => {
  if (!shuttingDown) shutdown(code ?? 0)
})
process.once('SIGINT', () => shutdown(0))
process.once('SIGTERM', () => shutdown(0))
