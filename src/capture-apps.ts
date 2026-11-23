import type { CaptureApp } from './types'

export interface AppSelection { appId: string; windowId: string; needsReselection: boolean }
export const emptyAppSelection = (): AppSelection => ({ appId: '', windowId: '', needsReselection: false })

export function chooseApp(app: CaptureApp): AppSelection {
  return { appId: app.id, windowId: app.windows.length === 1 ? app.windows[0].id : '', needsReselection: false }
}

export function reconcileAppSelection(apps: CaptureApp[], selected: AppSelection, initialize = false): AppSelection {
  const app = apps.find(item => item.id === selected.appId)
  if (app) {
    if (!selected.windowId || app.windows.some(window => window.id === selected.windowId)) return selected
    return { appId: app.id, windowId: '', needsReselection: true }
  }
  if (selected.appId || selected.windowId) return { appId: '', windowId: '', needsReselection: true }
  if (initialize && !selected.needsReselection && apps.length) return chooseApp(apps[0])
  return selected
}

export function filterApps(apps: CaptureApp[], query: string): CaptureApp[] {
  const search = query.trim().toLocaleLowerCase()
  return search ? apps.filter(app => `${app.name} ${app.windows.map(window => window.title).join(' ')}`.toLocaleLowerCase().includes(search)) : apps
}
