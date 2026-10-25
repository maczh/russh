import type { RuSSHApi } from '@shared/api'

export function getRuSSHClient(): RuSSHApi {
  return window.russhApi
}

export function getRuSSHDomainClient<TKey extends keyof RuSSHApi>(key: TKey): RuSSHApi[TKey] {
  return new Proxy({} as RuSSHApi[TKey], {
    get(_target, property) {
      return getRuSSHClient()[key][property as keyof RuSSHApi[TKey]]
    }
  })
}

export const russhClient: RuSSHApi = new Proxy({} as RuSSHApi, {
  get(_target, property) {
    return getRuSSHClient()[property as keyof RuSSHApi]
  }
})
