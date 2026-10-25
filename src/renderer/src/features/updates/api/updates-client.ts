import { getRuSSHDomainClient } from '@/features/shared/api/russh-client'
import type { UpdateState } from '@shared/types'

const baseClient = getRuSSHDomainClient('updates')

export const updatesClient = new Proxy(baseClient, {
  get(target, property) {
    if (property === 'check') {
      return async (): Promise<UpdateState> => {
        const state = await baseClient.check()
        if (state.phase !== 'checking') {
          return state
        }

        return new Promise<UpdateState>((resolve) => {
          let resolved = false

          const unsubscribe = baseClient.onStateChange((nextState) => {
            if (nextState.phase !== 'checking' && !resolved) {
              resolved = true
              clearTimeout(timeoutId)
              unsubscribe()
              resolve(nextState)
            }
          })

          const timeoutId = setTimeout(() => {
            if (!resolved) {
              resolved = true
              unsubscribe()
              baseClient.getState().then(resolve)
            }
          }, 15000)
        })
      }
    }

    return target[property as keyof typeof target]
  }
})
