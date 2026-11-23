import { convertFileSrc, invoke, isTauri } from '@tauri-apps/api/core'

export const desktop = isTauri()
export function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) return Promise.reject(new Error('Open the Windows desktop app to record and edit your projects.'))
  return invoke<T>(name, args)
}
export const mediaUrl = (path: string) => desktop ? convertFileSrc(path) : path
export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  return 'The operation failed. Try again.'
}
