import { describe, expect, it } from 'vitest'
import { chooseApp, emptyAppSelection, filterApps, reconcileAppSelection } from './capture-apps'
import type { CaptureApp } from './types'

const app = (id: number, windows: number[]): CaptureApp => ({ id: `process:${id}`, name: id === 41 ? 'Editor.exe' : 'Browser.exe', windows: windows.map(window => ({ id: `app:${id}:${window}`, title: `Document ${window}`, kind: 'window', width: 1280, height: 720 })) })

describe('app window selection', () => {
  it('requires a window choice for multiple windows and does not initialize another mode', () => {
    const apps = [app(41, [10, 20])]
    expect(reconcileAppSelection(apps, emptyAppSelection())).toEqual(emptyAppSelection())
    expect(reconcileAppSelection(apps, emptyAppSelection(), true)).toEqual({ appId: 'process:41', windowId: '', needsReselection: false })
    expect(chooseApp(app(42, [30])).windowId).toBe('app:42:30')
  })
  it('preserves the exact selected window when the refreshed order changes', () => {
    const selected = { ...chooseApp(app(41, [10, 20])), windowId: 'app:41:20' }
    expect(reconcileAppSelection([app(42, [30]), app(41, [20, 10])], selected, true)).toEqual(selected)
  })
  it('requires explicit reselection after window loss even if only one sibling remains', () => {
    const selected = { ...chooseApp(app(41, [10, 20])), windowId: 'app:41:20' }
    const missing = reconcileAppSelection([app(41, [10])], selected, true)
    expect(missing).toEqual({ appId: 'process:41', windowId: '', needsReselection: true })
    expect(reconcileAppSelection([app(41, [10, 20])], missing, true)).toEqual(missing)
    expect(chooseApp(app(41, [10]))).toEqual({ appId: 'process:41', windowId: 'app:41:10', needsReselection: false })
  })
  it('does not replace a missing process or automatically restore its later appearance', () => {
    const missing = reconcileAppSelection([app(42, [30])], chooseApp(app(41, [10])), true)
    expect(missing).toEqual({ appId: '', windowId: '', needsReselection: true })
    expect(reconcileAppSelection([app(41, [10]), app(42, [30])], missing, true)).toEqual(missing)
  })
  it('filters executable names and actual window titles without mutating the catalog', () => {
    const apps = [app(41, [10, 20]), app(42, [30])]
    expect(filterApps(apps, ' EDITOR.EXE ')).toEqual([apps[0]])
    expect(filterApps(apps, 'document 30')).toEqual([apps[1]])
    expect(filterApps(apps, 'missing')).toEqual([])
    expect(apps).toHaveLength(2)
  })
})
