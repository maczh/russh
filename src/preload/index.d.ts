import type { RuSSHApi } from '@shared/api'

declare global {
  interface Window {
    russhApi: RuSSHApi
  }
}

export {}
