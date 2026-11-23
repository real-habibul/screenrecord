import { expect, test, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import path from 'node:path'

let jpeg: string
test.beforeAll(() => {
  mkdirSync('artifacts/ui', { recursive: true })
  jpeg = execFileSync(process.env.SCREENRECORD_FFMPEG ?? path.resolve('src-tauri/resources/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=blue:size=480x270', '-frames:v', '1', '-c:v', 'mjpeg', '-threads', '1', '-f', 'image2pipe', 'pipe:1'], { windowsHide: true }).toString('base64')
})
async function mount(page: Page, holdInitialStatus = false) {
  await page.setViewportSize({ width: 340, height: 278 })
  await page.addInitScript(({ data, holdInitialStatus }) => {
    const w = window as any; w.isTauri = true
    let callback = 0; const callbacks = new Map<number, (value: unknown) => void>(); const listeners = new Map<string, number>()
    const state = { active: true, paused: false, phase: 'recording', cameraAvailable: true, elapsed: 12.25, projectId: 'recording-1' }
    w.__recorder = { state, generation: 1, sequence: 0, calls: [], holdFrame: false, finishFrame: null, holdStatus: holdInitialStatus, finishStatus: null, statusError: null, cameraError: null, actionError: null, holdStop: false, finishStop: null, frameInFlight: 0, maximumFrames: 0, position: { x: 10, y: 20 } }
    w.__recorder.emit = (event: string, payload: unknown) => callbacks.get(listeners.get(event)!)?.({ event, payload })
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
    w.__TAURI_INTERNALS__ = { transformCallback: (value: (data: unknown) => void) => { callbacks.set(++callback, value); return callback }, unregisterCallback: (id: number) => callbacks.delete(id), metadata: { currentWindow: { label: 'recording-controls' }, currentWebview: { label: 'recording-controls', windowLabel: 'recording-controls' } }, invoke: async (name: string, args: any) => {
      const control = w.__recorder; control.calls.push({ name, args })
      if (name === 'plugin:event|listen') { listeners.set(args.event, args.handler); return args.handler }
      if (name === 'plugin:event|unlisten') return
      if (name === 'get_recording_state') { const snapshot = structuredClone(state); if (control.holdStatus) await new Promise<void>(resolve => { control.finishStatus = resolve }); if (control.statusError) throw new Error(control.statusError); return snapshot }
      if (name === 'get_recording_camera_frame') {
        control.maximumFrames = Math.max(control.maximumFrames, ++control.frameInFlight)
        const frame = { generation: control.generation, sequence: ++control.sequence, mimeType: 'image/jpeg', data, active: state.active && !state.paused, error: control.cameraError }
        try { if (control.holdFrame) await new Promise<void>(resolve => { control.finishFrame = resolve }); return frame } finally { control.frameInFlight-- }
      }
      if (['pause_recording', 'resume_recording', 'stop_recording'].includes(name)) {
        if (args.projectId !== state.projectId) throw new Error('Recording changed. Refresh controls.')
        if (control.actionError) throw new Error(control.actionError)
        if (name === 'stop_recording') { state.phase = 'finalizing'; if (control.holdStop) await new Promise<void>(resolve => { control.finishStop = resolve }); state.active = false; state.phase = 'idle'; return {} }
        state.paused = name === 'pause_recording'; state.phase = state.paused ? 'paused' : 'recording'
        if (!state.paused) { control.generation++; control.sequence = 0 }
        return structuredClone(state)
      }
      if (name === 'restore_main_window' || name === 'plugin:window|start_dragging') return
      if (name === 'plugin:window|outer_position') return control.position
      if (name === 'plugin:window|set_position') { control.position = { x: args.value.position.x, y: args.value.position.y }; return }
      throw new Error(`Unexpected overlay command: ${name}`)
    } }
  }, { data: jpeg, holdInitialStatus })
  await page.goto('/?overlay=recording')
  if (!holdInitialStatus) await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeEnabled()
}

test('floating Stop cannot invoke without an acknowledged active project identity', async ({ page }) => {
  await mount(page, true); await expect.poll(() => page.evaluate(() => typeof (window as any).__recorder.finishStatus)).toBe('function')
  const stop = page.getByRole('button', { name: 'Stop recording', exact: true })
  await expect(stop).toBeDisabled(); await expect(page.getByRole('status')).toHaveText('Connecting...')
  await stop.evaluate(button => (button as HTMLButtonElement).click()); expect(await page.evaluate(() => (window as any).__recorder.calls.some((call: any) => call.name === 'stop_recording'))).toBe(false)
  await page.evaluate(() => { const state = (window as any).__recorder; state.holdStatus = false; state.finishStatus() }); await expect(stop).toBeEnabled(); await stop.click()
  expect(await page.evaluate(() => (window as any).__recorder.calls.find((call: any) => call.name === 'stop_recording').args.projectId)).toBe('recording-1')
})

test('floating controls use the existing capture camera and keep pause, resume and stop session scoped', async ({ page }) => {
  await mount(page)
  await expect(page.getByRole('img', { name: 'Camera view from the active recording' })).toBeVisible(); await expect(page.getByLabel('Recorded duration')).toHaveText('0:12')
  await page.getByRole('button', { name: 'Pause', exact: true }).click(); await expect(page.getByText('Camera paused', { exact: true })).toBeVisible(); await expect(page.getByRole('img')).toHaveCount(0)
  await page.getByRole('button', { name: 'Resume', exact: true }).click(); await expect(page.getByRole('img')).toBeVisible()
  await page.getByRole('button', { name: 'Show app', exact: true }).click()
  await page.clock.install({ time: new Date('2026-11-11T00:00:00Z') }); await page.clock.pauseAt(new Date('2026-11-11T00:00:01Z'))
  await page.evaluate(() => { (window as any).__recorder.holdStop = true }); await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeDisabled(); await expect(page.getByRole('status')).toHaveText('Finalizing')
  await page.clock.runFor(25_000); await expect(page.getByRole('status')).toHaveText('Finalizing'); await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeDisabled(); await expect(page.getByRole('img')).toHaveCount(0)
  await page.getByRole('button', { name: 'Stop recording', exact: true }).evaluate(button => (button as HTMLButtonElement).click()); expect(await page.evaluate(() => (window as any).__recorder.calls.filter((call: any) => call.name === 'stop_recording').length)).toBe(1)
  await page.evaluate(() => (window as any).__recorder.emit('recording-finalization-progress', { projectId: 'recording-1', progress: .42 })); await expect(page.getByRole('status')).toHaveText('Finalizing 42%')
  await page.evaluate(() => { const state = (window as any).__recorder; state.holdStop = false; state.finishStop() }); await expect(page.getByRole('status')).toHaveText('Recording stopped')
  const calls = await page.evaluate(() => (window as any).__recorder.calls)
  expect(calls.filter((call: any) => ['pause_recording', 'resume_recording', 'stop_recording'].includes(call.name)).every((call: any) => call.args.projectId === 'recording-1')).toBe(true)
  expect(calls.some((call: any) => call.name === 'restore_main_window')).toBe(true)
  expect(calls.some((call: any) => ['list_projects', 'list_capture_devices', 'start_camera_preview', 'start_recording'].includes(call.name))).toBe(false)
})

test('late camera frames stay hidden after pause and polling never overlaps', async ({ page }) => {
  await mount(page); await page.evaluate(() => { (window as any).__recorder.holdFrame = true })
  await expect.poll(() => page.evaluate(() => typeof (window as any).__recorder.finishFrame)).toBe('function')
  await page.getByRole('button', { name: 'Pause', exact: true }).click(); await page.evaluate(() => { const state = (window as any).__recorder; state.holdFrame = false; state.finishFrame() })
  await expect(page.getByText('Camera paused', { exact: true })).toBeVisible(); await expect(page.getByRole('img')).toHaveCount(0)
  await page.getByRole('button', { name: 'Resume', exact: true }).click(); await expect(page.getByRole('img')).toBeVisible()
  expect(await page.evaluate(() => (window as any).__recorder.maximumFrames)).toBe(1)
  await page.evaluate(() => { const control = (window as any).__recorder; Object.assign(control.state, { projectId: 'recording-2', elapsed: 0, cameraAvailable: false }); control.generation = 0; control.sequence = 0 })
  await expect(page.getByRole('img')).toHaveCount(0); await expect(page.getByLabel('Recorded duration')).toHaveText('0:00')
  await page.evaluate(() => (window as any).__recorder.emit('recording-error', { projectId: 'recording-1', error: 'Old capture disconnected' })); await expect(page.getByRole('alert')).toHaveCount(0)
  await page.evaluate(() => (window as any).__recorder.emit('recording-error', { projectId: 'recording-2', error: 'Current capture disconnected' })); await expect(page.getByRole('alert')).toHaveText('Current capture disconnected')
  await page.setViewportSize({ width: 340, height: 116 }); const stop = (await page.getByRole('button', { name: 'Stop recording', exact: true }).boundingBox())!; expect(stop.y + stop.height).toBeLessThanOrEqual(116)
  await page.getByRole('button', { name: 'Pause', exact: true }).click(); expect(await page.evaluate(() => (window as any).__recorder.calls.filter((call: any) => call.name === 'pause_recording').at(-1).args.projectId)).toBe('recording-2')
})

test('status and action failures remain visible while Stop stays available', async ({ page }) => {
  await mount(page)
  await page.evaluate(() => { (window as any).__recorder.statusError = 'Recorder status temporarily unavailable' }); await expect(page.getByRole('alert')).toContainText('Recorder status temporarily unavailable'); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.evaluate(() => { const state = (window as any).__recorder; state.statusError = null; state.actionError = 'Pause failed' }); await page.getByRole('button', { name: 'Pause', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('Pause failed'); await expect.poll(() => page.evaluate(() => (window as any).__recorder.calls.filter((call: any) => call.name === 'get_recording_state').length)).toBeGreaterThan(3); await expect(page.getByRole('alert')).toHaveText('Pause failed')
  await page.evaluate(() => (window as any).__recorder.emit('recording-error', 'Capture disconnected')); await expect(page.getByRole('alert')).toHaveText('Capture disconnected')
  await page.evaluate(() => { (window as any).__recorder.actionError = null }); await page.getByRole('button', { name: 'Stop recording', exact: true }).click(); await expect(page.getByRole('status')).toHaveText('Recording stopped')
})

test('floating controls fit both themes and support pointer and keyboard positioning', async ({ page }) => {
  await mount(page); await expect(page.getByRole('img')).toBeVisible()
  const move = page.getByRole('button', { name: 'Move recording controls', exact: true }); await move.click()
  expect(await page.evaluate(() => (window as any).__recorder.calls.some((call: any) => call.name === 'plugin:window|start_dragging'))).toBe(true)
  await move.focus(); await move.press('ArrowRight'); await move.press('Shift+ArrowDown')
  await expect.poll(() => page.evaluate(() => (window as any).__recorder.calls.filter((call: any) => call.name === 'plugin:window|set_position').length)).toBe(2)
  expect(await page.evaluate(() => (window as any).__recorder.position)).toEqual({ x: 11, y: 30 })
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { localStorage.setItem('screenrecord-theme', theme); window.dispatchEvent(new StorageEvent('storage')) }, theme)
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    const stop = (await page.getByRole('button', { name: 'Stop recording', exact: true }).boundingBox())!; expect(stop.y + stop.height).toBeLessThanOrEqual(278)
    await page.screenshot({ path: `artifacts/ui/recording-controls-${theme}.png` })
  }
})
