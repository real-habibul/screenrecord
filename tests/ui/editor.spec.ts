import { expect, test, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, existsSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { audioFixture, mockedDesktop, prepareDesktopMedia } from './fixtures/desktop'
import { serveMedia } from './fixtures/media'

const screenshotDir = path.resolve('artifacts/ui')
function writeEvidence(file: string, measurements: unknown) {
  mkdirSync(path.dirname(file), { recursive: true })
  writeFileSync(file, JSON.stringify(measurements, null, 2))
}
const pcmTapSource = "class Tap extends AudioWorkletProcessor {process(inputs,outputs){let peak=0,count=0,energy=0;for(const c of inputs[0]??[]){count+=c.length;for(const n of c){peak=Math.max(peak,Math.abs(n));energy+=n*n;}}this.port.postMessage({peak,count,energy});for(const c of outputs[0]??[])c.fill(0);return true}}registerProcessor('pcm-tap',Tap)"
test.beforeAll(prepareDesktopMedia)

test('browser view is honest and theme remains usable', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('status')).toContainText('require the Windows desktop app')
  await expect(page.getByRole('button', { name: 'New recording' }).first()).toBeDisabled()
  await expect(page.getByText('Stored on this computer', { exact: true })).toHaveCount(0)
  const theme = await page.locator('html').getAttribute('data-theme')
  await page.getByRole('button', { name: /Switch to/ }).click()
  await expect(page.locator('html')).not.toHaveAttribute('data-theme', theme!)
})

test('appearance presets apply as one undo and save only styles while preserving all source edits', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { videoClips: [{ id: 'keep', start: 1, end: 9 }], crop: { x: .1, y: .1, width: .8, height: .8 }, music: [{ id: 'song', name: 'Keep music', path: '/test-media/audio.wav', sourceDuration: 12, sourceStart: 1, start: 2, duration: 4, volume: .3, fadeIn: 0, fadeOut: 0 }], subtitles: [{ id: 'cue', start: 1, end: 2, text: 'Keep these words' }], redactions: [{ id: 'mask', start: 3, end: 5, x: .1, y: .1, width: .2, height: .2, mode: 'solid', strength: 12 }], audioProcessing: { microphoneGain: 2, systemGain: 3, ducking: { enabled: false, amountDb: 9 } } } }))
  const original = await page.evaluate(() => (window as any).__test.currentProject().edits)
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByLabel('Appearance preset', { exact: true }).selectOption('builtin-split'); await page.getByRole('button', { name: 'Apply', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.camera.layout)).toBe('split')
  const applied = await page.evaluate(() => (window as any).__test.saved.edits)
  for (const key of ['trimStart', 'trimEnd', 'videoClips', 'crop', 'zooms', 'music', 'subtitles', 'redactions', 'systemVolume', 'microphoneVolume', 'audioProcessing']) expect(applied[key]).toEqual(original[key])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled(); await expect(page.getByLabel('Aspect ratio')).toHaveValue('source')
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(page.getByLabel('Aspect ratio')).toHaveValue('16:9')
  await page.getByRole('button', { name: 'Save current...', exact: true }).click(); await page.getByLabel('Preset name', { exact: true }).fill('My presenter layout'); await page.getByRole('button', { name: 'Save preset', exact: true }).click()
  await expect(page.getByRole('status').filter({ hasText: 'Preset saved.' })).toBeVisible()
  const preset = await page.evaluate(() => (window as any).__test.presets[0])
  expect(preset.name).toBe('My presenter layout'); expect(Object.keys(preset.settings).sort()).toEqual(['fitMode', 'backgroundMode', 'backgroundBlur', 'aspectRatio', 'background', 'camera', 'cornerRadius', 'cursorSize', 'cursorSmoothing', 'padding', 'showCursor', 'showClicks', 'subtitleStyle'].sort())
  expect(preset.settings.camera.layout).toBe('split'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeEnabled()
  await page.getByRole('tab', { name: 'Camera', exact: true }).click(); await expect(page.getByLabel('Camera layout')).toHaveValue('split'); await expect(page.getByLabel('Shape', { exact: true })).toHaveCount(0); await expect(page.getByLabel(/Horizontal position/)).toHaveCount(0); await expect(page.getByLabel(/Camera share/)).toBeVisible()
})

test('custom preset errors are retryable and deletion confirms without changing project appearance', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { (window as any).__test.presetsError = 'Custom presets file is corrupt' })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByRole('alert')).toContainText('Custom presets file is corrupt')
  await page.getByLabel('Appearance preset', { exact: true }).selectOption('builtin-screen'); await page.getByRole('button', { name: 'Apply', exact: true }).click(); await expect(page.getByLabel(/Padding/)).toHaveValue('0')
  await expect(page.getByRole('button', { name: 'Save current...', exact: true })).toBeDisabled(); await page.evaluate(() => { (window as any).__test.presetsError = null }); await page.getByRole('button', { name: 'Retry presets', exact: true }).click()
  await page.getByRole('button', { name: 'Save current...', exact: true }).click(); await page.getByLabel('Preset name').fill('Retry preset'); await page.evaluate(() => { (window as any).__test.presetSaveError = 'Settings disk is full' }); await page.getByRole('button', { name: 'Save preset', exact: true }).click(); await expect(page.getByRole('alert')).toContainText('Settings disk is full'); await expect(page.getByLabel('Preset name')).toHaveValue('Retry preset')
  await page.evaluate(() => { (window as any).__test.presetSaveError = null }); await page.getByRole('button', { name: 'Save preset', exact: true }).click(); await page.getByRole('button', { name: 'Delete preset...', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Delete appearance preset?' }); await expect(dialog).toContainText('Retry preset'); await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused(); await page.keyboard.press('Escape')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_appearance_preset'))).toBe(false)
  await page.getByRole('button', { name: 'Delete preset...', exact: true }).click(); await page.evaluate(() => { (window as any).__test.presetDeleteError = 'Settings are read only' }); await dialog.getByRole('button', { name: 'Delete preset', exact: true }).click(); await expect(dialog.getByRole('alert')).toContainText('Settings are read only')
  await page.evaluate(() => { (window as any).__test.presetDeleteError = null }); await dialog.getByRole('button', { name: 'Delete preset', exact: true }).click(); await expect(dialog).toBeHidden(); await expect(page.getByLabel(/Padding/)).toHaveValue('0'); expect(await page.evaluate(() => (window as any).__test.presets)).toEqual([])
  await expect(page.getByRole('button', { name: 'Save current...', exact: true })).toBeFocused()
})

test('legacy click appearance defaults on, supports keyboard undo and saves through presets and reopen', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Cursor', exact: true }).click()
  const clicks = page.getByRole('checkbox', { name: 'Show click animations', exact: true }); const cursor = page.getByRole('checkbox', { name: 'Show cursor', exact: true })
  await expect(clicks).toBeChecked(); await expect(clicks).toBeEnabled(); await clicks.focus(); await page.keyboard.press('Space'); await expect(clicks).not.toBeChecked()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.showClicks)).toBe(false)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(clicks).toBeChecked(); await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(clicks).not.toBeChecked()
  await cursor.uncheck(); await expect(clicks).toBeDisabled(); await cursor.check(); await expect(clicks).toBeEnabled(); await expect(clicks).not.toBeChecked()
  await page.setViewportSize({ width: 900, height: 640 }); await clicks.focus(); await page.screenshot({ path: path.resolve('artifacts/clicks/cursor-dark-900.png') }); await page.getByRole('button', { name: 'Switch to light theme' }).click(); await clicks.focus(); await page.screenshot({ path: path.resolve('artifacts/clicks/cursor-light-900.png') })
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false)
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await page.getByRole('button', { name: 'Save current...', exact: true }).click(); await page.getByLabel('Preset name', { exact: true }).fill('Quiet pointer'); await page.getByRole('button', { name: 'Save preset', exact: true }).click()
  expect(await page.evaluate(() => (window as any).__test.presets[0].settings.showClicks)).toBe(false)
  await page.getByRole('tab', { name: 'Cursor', exact: true }).click(); await clicks.check(); await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); const preset = await page.evaluate(() => (window as any).__test.presets[0].id); await page.getByLabel('Appearance preset', { exact: true }).selectOption(preset); await page.getByRole('button', { name: 'Apply', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.showClicks)).toBe(false)
  const library = page.getByRole('button', { name: 'Projects', exact: true }); if (await library.getAttribute('aria-expanded') !== 'true') await library.click(); await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Cursor', exact: true }).click(); await expect(clicks).not.toBeChecked()
})

test('camera-only preview hides screen through gaps, EOF, disabled camera and pending decoder seeks', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: '/test-media/camera-short.mp4', cameraIntervals: [{ start: 0, end: .2 }, { start: .5, end: .8 }], edits: { zooms: [], padding: 32, camera: { enabled: true, layout: 'bubble', x: .75, y: .64, width: .2, shape: 'circle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Camera', exact: true }).click()
  const pixel = () => page.locator('canvas').evaluate(element => [...element.getContext('2d')!.getImageData(Math.floor(element.width * .4), Math.floor(element.height * .4), 1, 1).data].slice(0, 3))
  const seek = (value: number) => page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, value)
  await expect.poll(async () => (await pixel())[1]).toBeGreaterThan(100)
  await page.evaluate(() => { const video = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!; (window as any).__holdSeek = true; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__holdSeek }); video.currentTime = .3 })
  await page.getByLabel('Camera layout').selectOption('fullscreen'); await expect.poll(pixel).toEqual([40, 53, 68]); await expect(page.getByRole('button', { name: 'Move camera', exact: true })).toHaveCount(0)
  await page.evaluate(() => { (window as any).__holdSeek = false; document.querySelector('video[data-track="screen recording"]')!.dispatchEvent(new Event('seeked')) })
  await expect(page.getByText('Camera unavailable', { exact: true })).toBeVisible()
  await seek(.1); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
  await page.evaluate(() => { (window as any).__holdSeek = true }); await page.getByLabel('Show webcam', { exact: true }).uncheck(); await expect.poll(pixel).toEqual([40, 53, 68]); await page.evaluate(() => { (window as any).__holdSeek = false }); await page.getByLabel('Show webcam', { exact: true }).check()
  await seek(.3); await expect.poll(pixel).toEqual([40, 53, 68]); await seek(.6); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220); await seek(1.2); await expect.poll(pixel).toEqual([40, 53, 68])
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await expect(page.getByRole('button', { name: 'Draw on preview', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'Add mask', exact: true })).toBeDisabled(); await page.getByRole('button', { name: 'Switch to bubble layout', exact: true }).click(); await expect(page.getByRole('button', { name: 'Draw on preview', exact: true })).toBeEnabled()
})

test('split camera reserves its pane across EOF and mirrors both orientations without exposing drag handles', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: '/test-media/camera-colors.mp4', edits: { aspectRatio: '16:9', zooms: [], padding: 32, camera: { enabled: true, layout: 'split', x: 0, y: 0, width: .3, shape: 'circle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Camera', exact: true }).click()
  const pixel = (x: number, y: number) => page.locator('canvas').evaluate((element, point) => [...element.getContext('2d')!.getImageData(Math.floor(element.width * point.x), Math.floor(element.height * point.y), 1, 1).data].slice(0, 3), { x, y })
  await expect.poll(async () => (await pixel(.7542, .5))[0]).toBeGreaterThan(220); await expect.poll(async () => (await pixel(.926, .5))[2]).toBeGreaterThan(220); await expect.poll(async () => (await pixel(.2, .5))[1]).toBeGreaterThan(100)
  await expect(page.getByRole('button', { name: 'Move camera', exact: true })).toHaveCount(0); await expect(page.getByLabel('Shape', { exact: true })).toHaveCount(0)
  await page.getByLabel('Mirror webcam', { exact: true }).check(); await expect.poll(async () => (await pixel(.7542, .5))[2]).toBeGreaterThan(220); await page.getByLabel('Mirror webcam', { exact: true }).uncheck()
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByRole('button', { name: 'Draw on preview', exact: true }).click()
  const bounds = (await page.locator('canvas').boundingBox())!
  await page.mouse.move(bounds.x + bounds.width * (32 + .2 * 1282) / 1920, bounds.y + bounds.height * (180 + .2 * 720) / 1080); await page.mouse.down(); await page.mouse.move(bounds.x + bounds.width * (32 + .4 * 1282) / 1920, bounds.y + bounds.height * (180 + .4 * 720) / 1080, { steps: 4 }); await page.mouse.up()
  expect(Number(await page.getByLabel('Region left %', { exact: true }).inputValue())).toBeCloseTo(20, 0); expect(Number(await page.getByLabel('Region width %', { exact: true }).inputValue())).toBeCloseTo(20, 0)
  await expect.poll(() => pixel((32 + .3 * 1282) / 1920, (180 + .3 * 720) / 1080)).toEqual([0, 0, 0]); await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await page.setViewportSize({ width: 1920, height: 1080 })
  await page.screenshot({ path: 'artifacts/ui/presets-split-dark.png' }); await page.getByRole('button', { name: 'Switch to light theme', exact: true }).click(); await page.screenshot({ path: 'artifacts/ui/presets-split-light.png' })
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await expect.poll(async () => (await pixel(.25, .85))[0]).toBeGreaterThan(220); await expect.poll(async () => (await pixel(.75, .85))[2]).toBeGreaterThan(220); await expect.poll(async () => (await pixel(.5, .3))[1]).toBeGreaterThan(100)
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '1.2'; element.dispatchEvent(new Event('input', { bubbles: true })) }); await expect.poll(() => pixel(.5, .85)).toEqual([40, 53, 68]); await expect.poll(async () => (await pixel(.5, .3))[0]).toBeGreaterThan(220)
})

test('missing camera rejects camera-only presets and late custom-load results cannot replace a reopened panel', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { const state = (window as any).__test; state.holdPresets = true; state.presets = [{ id: 'old-preset', name: 'Stale preset result', settings: state.currentProject().edits }]; state.patchProject({ cameraPath: null, edits: { camera: { enabled: false, layout: 'fullscreen', x: .7, y: .7, width: .2, shape: 'circle', mirror: true } } }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishPresets)).toBe('function')
  await page.getByRole('tab', { name: 'Camera', exact: true }).click(); await page.evaluate(() => { (window as any).__test.holdPresets = false; (window as any).__test.presets = [] }); await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await page.evaluate(() => (window as any).__test.finishPresets()); await expect(page.getByRole('option', { name: 'Stale preset result', exact: true })).toHaveCount(0)
  await page.getByLabel('Appearance preset', { exact: true }).selectOption('builtin-camera'); await expect(page.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled(); await expect(page.getByText('This preset requires recorded camera video.', { exact: false })).toBeVisible(); await expect(page.getByRole('button', { name: 'New recording with camera', exact: true })).toBeEnabled()
  await expect.poll(() => page.locator('canvas').evaluate(element => [...element.getContext('2d')!.getImageData(element.width / 2, element.height / 2, 1, 1).data].slice(0, 3))).toEqual([40, 53, 68])
  await page.getByLabel('Appearance preset', { exact: true }).selectOption('builtin-split'); await page.getByRole('button', { name: 'Apply', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.camera)).toMatchObject({ layout: 'split', enabled: false })
  await expect(page.getByRole('button', { name: 'Delete preset...', exact: true })).toHaveCount(0); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_appearance_preset'))).toBe(false)
})

test('privacy edits clear cached captions and unmasked pixels immediately during pending decoded seeks', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, edits: { zooms: [], padding: 0, cornerRadius: 0, showCursor: false, subtitles: [{ id: 'private', start: 0, end: 2, text: 'Private caption to remove' }], redactions: [{ id: 'field', start: 0, end: 2, x: .2, y: .2, width: .2, height: .2, mode: 'blur', strength: 1 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const pixel = (x = .3, y = .3) => page.locator('canvas').evaluate((element, point) => [...element.getContext('2d')!.getImageData(Math.floor(element.width * point.x), Math.floor(element.height * point.y), 1, 1).data].slice(0, 3), { x, y })
  await expect.poll(async () => (await pixel())[0]).toBeGreaterThan(220)
  await page.evaluate(() => { (window as any).__holdPrivateSeek = false; const video = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__holdPrivateSeek }) })
  const hold = () => page.evaluate(() => { (window as any).__holdPrivateSeek = true })
  const release = () => page.evaluate(() => { (window as any).__holdPrivateSeek = false; document.querySelector('video[data-track="screen recording"]')!.dispatchEvent(new Event('seeked')) })
  await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await hold(); await page.getByLabel('Show subtitles in video', { exact: true }).uncheck(); await expect.poll(pixel).toEqual([40, 53, 68]); await release(); await expect.poll(async () => (await pixel())[1]).toBeGreaterThan(100)
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('field')
  await hold(); await page.getByLabel('Effect', { exact: true }).selectOption('solid'); await expect.poll(pixel).toEqual([40, 53, 68]); await release(); await expect.poll(pixel).toEqual([0, 0, 0])
  await expect.poll(async () => (await pixel(.5, .3))[1]).toBeGreaterThan(100); await hold(); await page.getByLabel('Region width %', { exact: true }).fill('50'); await page.getByLabel('Region width %', { exact: true }).press('Tab'); await expect.poll(() => pixel(.5, .3)).toEqual([40, 53, 68]); await release(); await expect.poll(() => pixel(.5, .3)).toEqual([0, 0, 0])
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await hold(); await page.getByLabel('Crop recording', { exact: true }).check(); await expect.poll(pixel).toEqual([40, 53, 68]); await release(); await expect.poll(pixel).toEqual([0, 0, 0])
})

const screenPixel = (page: Page, x: number, y: number) => page.locator('canvas').evaluate((element, point) => [...element.getContext('2d')!.getImageData(Math.floor(element.width * point.x), Math.floor(element.height * point.y), 1, 1).data].slice(0, 3), { x, y })

test('screen sizing Fit preserves the whole source while Fill covers portrait and square with undo and saved reopen', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/camera-colors.mp4', cameraPath: null, width: 320, height: 180, duration: 1, cursor: [], edits: { trimStart: 0, trimEnd: 1, aspectRatio: '9:16', zooms: [], padding: 0, cornerRadius: 0, background: '#202020', showCursor: false } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); const sizing = page.getByLabel('Screen sizing', { exact: true })
  await expect(sizing).toHaveValue('fit'); await expect.poll(() => screenPixel(page, .25, .1)).toEqual([32, 32, 32]); await expect.poll(async () => (await screenPixel(page, .25, .5))[0]).toBeGreaterThan(220)
  await sizing.selectOption('fill'); await expect.poll(async () => (await screenPixel(page, .25, .1))[0]).toBeGreaterThan(220); await expect.poll(async () => (await screenPixel(page, .75, .9))[2]).toBeGreaterThan(220)
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.fitMode)).toBe('fill'); await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(sizing).toHaveValue('fit'); await expect.poll(() => screenPixel(page, .25, .1)).toEqual([32, 32, 32])
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(sizing).toHaveValue('fill'); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.fitMode)).toBe('fill')
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(sizing).toHaveValue('fill'); await page.getByLabel('Aspect ratio').selectOption('1:1'); await expect.poll(async () => (await screenPixel(page, .25, .1))[0]).toBeGreaterThan(220)
  await page.getByLabel('Aspect ratio').selectOption('9:16'); await page.getByRole('button', { name: 'Projects', exact: true }).click(); await page.setViewportSize({ width: 900, height: 640 }); mkdirSync('artifacts/vertical-0.5.0', { recursive: true })
  for (const theme of ['dark', 'light']) { await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme); await sizing.focus(); await expect(sizing).toBeInViewport(); expect(await sizing.evaluate(element => getComputedStyle(element).outlineStyle)).toBe('solid'); await page.screenshot({ path: `artifacts/vertical-0.5.0/stage1-${theme}-900x640.png` }) }
})

test('Fill stays inside manual crop, projects source masks and clears old pixels during a pending sizing change', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/camera-colors.mp4', cameraPath: null, width: 320, height: 180, duration: 1, cursor: [], edits: { fitMode: 'fill', trimStart: 0, trimEnd: 1, aspectRatio: '9:16', crop: { x: .5, y: 0, width: .5, height: 1 }, zooms: [], padding: 0, cornerRadius: 0, background: '#202020', showCursor: false, redactions: [{ id: 'kept-mask', start: 0, end: 1, x: .65, y: .2, width: .2, height: .6, mode: 'solid', strength: 12 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .1, .1))[2]).toBeGreaterThan(220); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([0, 0, 0])
  await page.evaluate(() => { (window as any).__holdSizingSeek = true; const video = document.querySelector('video[data-track="screen recording"]')!; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__holdSizingSeek }) })
  await page.getByLabel('Screen sizing', { exact: true }).selectOption('fit'); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([32, 32, 32]); await expect(page.getByRole('button', { name: 'Move mask', exact: true })).toHaveCount(0)
  await page.evaluate(() => { (window as any).__holdSizingSeek = false; document.querySelector('video[data-track="screen recording"]')!.dispatchEvent(new Event('seeked')) }); await page.getByLabel('Screen sizing', { exact: true }).selectOption('fill'); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([0, 0, 0])
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByRole('button', { name: 'Draw on preview', exact: true }).click(); const box = (await page.locator('canvas').boundingBox())!
  await page.mouse.move(box.x + box.width * .2, box.y + box.height * .2); await page.mouse.down(); await page.mouse.move(box.x + box.width * .4, box.y + box.height * .4, { steps: 4 }); await page.mouse.up()
  expect(Number(await page.getByLabel('Region left %', { exact: true }).inputValue())).toBeCloseTo(65.625, 0); expect(Number(await page.getByLabel('Region width %', { exact: true }).inputValue())).toBeCloseTo(6.25, 0)
})

test('Fill uses the split screen pane and keeps camera gaps and camera-only output private', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: '/test-media/camera-short.mp4', cameraIntervals: [{ start: 0, end: .2 }, { start: .5, end: .8 }], edits: { fitMode: 'fill', aspectRatio: '9:16', zooms: [], padding: 32, cornerRadius: 0, showCursor: false, camera: { enabled: true, layout: 'split', x: .75, y: .64, width: .3, shape: 'circle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .5, .1))[1]).toBeGreaterThan(100); await expect.poll(async () => (await screenPixel(page, .5, .85))[2]).toBeGreaterThan(220)
  const seek = (value: number) => page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, value)
  await seek(.3); await expect.poll(() => screenPixel(page, .5, .85)).toEqual([40, 53, 68]); await expect.poll(async () => (await screenPixel(page, .5, .1))[1]).toBeGreaterThan(100)
  await seek(.6); await expect.poll(async () => (await screenPixel(page, .5, .85))[2]).toBeGreaterThan(220); await seek(1.2); await expect.poll(() => screenPixel(page, .5, .85)).toEqual([40, 53, 68])
  await page.getByRole('tab', { name: 'Camera', exact: true }).click(); await page.getByLabel('Camera layout').selectOption('fullscreen'); await expect.poll(() => screenPixel(page, .5, .1)).toEqual([40, 53, 68])
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await expect(page.getByLabel('Screen sizing', { exact: true })).toBeDisabled(); await page.getByRole('button', { name: 'Switch to bubble layout', exact: true }).click(); await expect(page.getByLabel('Screen sizing', { exact: true })).toBeEnabled(); await expect.poll(async () => (await screenPixel(page, .5, .1))[0]).toBeGreaterThan(220)
})

test('manual framing captures the actual paused frame, updates nearby keys, and survives undo and saved reopen', async ({ page }) => {
  await page.addInitScript(() => { const original = window.requestAnimationFrame; window.requestAnimationFrame = callback => original.call(window, () => callback(0)) })
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, cursor: [], edits: { zooms: [], fitMode: 'fill', aspectRatio: '9:16' } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await seekEditor(page, 4.2); await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  const video = page.locator('video[data-track="screen recording"]')
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(4.45); await expect(page.getByLabel('Playhead position in seconds')).toHaveValue('4.2')
  await page.getByRole('button', { name: 'Set keyframe at playhead' }).click(); const actual = await video.evaluate(element => (element as HTMLVideoElement).currentTime)
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.[0]?.time)).toBe(actual); expect(actual).toBeGreaterThan(4.45)
  await page.getByLabel('Framing horizontal %').fill('75'); await page.getByLabel('Framing horizontal %').blur(); await page.getByLabel('Framing scale', { exact: true }).fill('2'); await page.getByLabel('Framing scale', { exact: true }).blur()
  await page.getByRole('button', { name: 'Set keyframe at playhead' }).click(); await expect(page.locator('#reframe-key option')).toHaveCount(1)
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.[0]?.scale)).toBe(2)
  await page.getByRole('button', { name: 'Delete framing keyframe' }).click(); await expect(page.locator('#reframe-key')).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.locator('#reframe-key option')).toHaveCount(2); await page.locator('#reframe-key').selectOption({ index: 1 }); await expect(page.getByLabel('Framing horizontal %')).toHaveValue('75')
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.length)).toBe(1); await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.locator('#reframe-key').selectOption({ index: 1 }); await expect(page.getByLabel('Framing scale', { exact: true })).toHaveValue('2')
  await page.getByRole('button', { name: 'Projects', exact: true }).click(); await page.setViewportSize({ width: 900, height: 640 })
  for (const theme of ['dark', 'light']) { await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme); await page.getByLabel('Framing horizontal %').focus(); await expect(page.getByLabel('Framing horizontal %')).toBeInViewport(); await page.screenshot({ path: `artifacts/vertical-0.5.0/stage2-${theme}-900x640.png` }) }
})

test('manual framing interpolates decoded pixels and clears them synchronously while a seek is pending', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/camera-colors.mp4', cameraPath: null, width: 320, height: 180, duration: 1, cursor: [], edits: { fitMode: 'fill', trimEnd: 1, aspectRatio: '9:16', zooms: [], padding: 0, cornerRadius: 0, background: '#202020', showCursor: false, reframeKeyframes: [{ id: 'left', time: 0, x: .2, y: .5, scale: 1 }, { id: 'right', time: .8, x: .8, y: .5, scale: 1 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .75, .5))[0]).toBeGreaterThan(220)
  await seekEditor(page, .4); await expect.poll(async () => (await screenPixel(page, .25, .5))[0]).toBeGreaterThan(220); await expect.poll(async () => (await screenPixel(page, .75, .5))[2]).toBeGreaterThan(220)
  await page.locator('#reframe-key').selectOption('right'); await expect.poll(async () => (await screenPixel(page, .25, .5))[2]).toBeGreaterThan(220)
  await page.evaluate(() => { const video = document.querySelector('video[data-track="screen recording"]')!; (window as any).__heldReframeSeek = true; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__heldReframeSeek }) })
  await page.getByLabel('Framing horizontal %').fill('20'); await page.getByLabel('Framing horizontal %').blur(); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([32, 32, 32]); await expect(page.getByRole('button', { name: 'Move mask', exact: true })).toHaveCount(0)
  await page.evaluate(() => { (window as any).__heldReframeSeek = false; document.querySelector('video[data-track="screen recording"]')!.dispatchEvent(new Event('seeked')) }); await seekEditor(page, .8); await expect.poll(async () => (await screenPixel(page, .75, .5))[0]).toBeGreaterThan(220)
})

test('manual framing keeps hidden source keys without exposing cut frames and adds at the clamped visible frame', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, cursor: [], duration: 6, edits: { trimEnd: 5, videoClips: [{ id: 'first', start: 0, end: 1 }, { id: 'last', start: 4, end: 5 }], zooms: [], padding: 0, cornerRadius: 0, showCursor: false, reframeKeyframes: [{ id: 'removed', time: 1.5, x: .2, y: .5, scale: 2 }, { id: 'after-trim', time: 5.5, x: .8, y: .5, scale: 1 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.locator('#reframe-key').selectOption('removed'); await expect(page.getByText('This keyframe is outside kept video.', { exact: false })).toBeVisible()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).currentTime)).toBe(4); await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await page.getByRole('button', { name: 'Set keyframe at playhead' }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.map((key: any) => key.time))).toEqual([1.5, 4, 5.5])
  await page.locator('#reframe-key').selectOption('after-trim'); await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).currentTime)).toBeLessThan(5); await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.map((key: any) => key.time))).toEqual([1.5, 5.5])
})

test('manual framing waits for a decoded seek and discards pending additions when a project changes', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { const t = (window as any).__test; t.patchProject({ cameraPath: null, edits: { zooms: [] } }); t.addProject('other', 'Other recording'); t.emit('library-updated') })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await seekEditor(page, 1)
  const hold = () => page.evaluate(() => { const video = document.querySelector('video[data-track="screen recording"]')!; (window as any).__heldReframeSeek = true; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__heldReframeSeek }); (window as any).__oldReframeVideo = video })
  await hold(); await page.getByRole('button', { name: 'Set keyframe at playhead' }).click(); await expect(page.getByRole('button', { name: 'Waiting for frame...' })).toBeDisabled(); expect(await page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.length ?? 0)).toBe(0)
  await page.evaluate(() => { (window as any).__heldReframeSeek = false; (window as any).__oldReframeVideo.dispatchEvent(new Event('seeked')) }); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.reframeKeyframes?.[0]?.time)).toBe(1)
  await seekEditor(page, 2); await hold(); await page.getByRole('button', { name: 'Set keyframe at playhead' }).click(); await expect(page.getByRole('button', { name: 'Waiting for frame...' })).toBeDisabled(); await page.getByRole('button', { name: /Other recording/ }).click()
  await page.evaluate(() => { (window as any).__heldReframeSeek = false; (window as any).__oldReframeVideo.dispatchEvent(new Event('seeked')) }); await expect(page.locator('#reframe-key')).toHaveCount(0); await expect(page.getByRole('button', { name: 'Set keyframe at playhead' })).toBeEnabled(); expect(await page.evaluate(() => (window as any).__test.currentProject().edits.reframeKeyframes.map((key: any) => key.time))).toEqual([1])
})

test('video background preserves one-pixel privacy, opaque edges and bounded buffers without stale frame caching', async ({ page }) => {
  await page.goto('/')
  const evidence = await page.evaluate(async () => {
    const { VideoBackground } = await import('/src/video-background.ts' as string)
    const source = document.createElement('canvas'); source.width = 3; source.height = 1
    const ctx = source.getContext('2d')!; ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 3, 1); ctx.fillStyle = 'blue'; ctx.fillRect(1, 0, 1, 1); ctx.fillStyle = 'lime'; ctx.fillRect(2, 0, 1, 1)
    const foreground = document.createElement('canvas'); foreground.width = 540; foreground.height = 960; foreground.getContext('2d')!.drawImage(source, 1, 0, 1, 1, 0, 0, 540, 960)
    const foregroundEdges = [[0, 0], [539, 0], [0, 959], [539, 959]].map(([x, y]) => [...foreground.getContext('2d')!.getImageData(x, y, 1, 1).data])
    const background = new VideoBackground(); const view = { sx: 1, sy: 0, sw: 1, sh: 1, width: 1080, height: 1920 }
    background.drawForeground(foreground.getContext('2d')!, source, view, { x: 0, y: 0, width: 540, height: 960 })
    const isolatedForeground = [[0, 0], [539, 0], [0, 959], [539, 959]].map(([x, y]) => [...foreground.getContext('2d')!.getImageData(x, y, 1, 1).data])
    const result = background.draw(source, view, 64) as HTMLCanvasElement
    const pixels = () => [[0, 0], [result.width - 1, 0], [0, result.height - 1], [result.width - 1, result.height - 1], [result.width / 2, result.height / 2]].map(([x, y]) => [...result.getContext('2d')!.getImageData(x, y, 1, 1).data])
    const basePixels = [[0, 0], [539, 0]].map(([x, y]) => [...(background as any).source.getContext('2d').getImageData(x, y, 1, 1).data]); const first = pixels(); ctx.fillStyle = 'lime'; ctx.fillRect(1, 0, 1, 1); background.draw(source, view, 4); const moved = pixels()
    background.clear(); const cleared = pixels(); const width = result.width; const height = result.height
    source.width = 2051; source.height = 2051; ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 2051, 2051); ctx.fillStyle = 'blue'; ctx.fillRect(1, 1, 2049, 2049)
    const started = performance.now(); const tiled = background.draw(source, { sx: 1, sy: 1, sw: 2049, sh: 2049, width: 2160, height: 2160 }, 64); const drawMs = performance.now() - started
    const data = tiled.getContext('2d')!.getImageData(0, 0, tiled.width, tiled.height).data; let red = 0, green = 0, blue = 255, alpha = 255
    for (let i = 0; i < data.length; i += 4) { red = Math.max(red, data[i]); green = Math.max(green, data[i + 1]); blue = Math.min(blue, data[i + 2]); alpha = Math.min(alpha, data[i + 3]) }
    background.release()
    return { foregroundEdges, isolatedForeground, first, basePixels, moved, cleared, width, height, tiled: { red, green, blue, alpha, drawMs }, released: result.width === 0 && result.height === 0 }
  })
  writeEvidence('artifacts/vertical-0.5.0/background-pixel-evidence.json', evidence)
  expect(evidence.isolatedForeground).toEqual(Array(4).fill([0, 0, 255, 255])); expect(evidence.first).toEqual(Array(5).fill([0, 0, 255, 255])); expect(evidence.moved).toEqual(Array(5).fill([0, 255, 0, 255])); expect(evidence.cleared).toEqual(Array(5).fill([0, 0, 0, 255])); expect(Math.max(evidence.width, evidence.height)).toBeLessThanOrEqual(960); expect(evidence.released).toBe(true); expect(evidence.tiled).toMatchObject({ red: 0, green: 0, blue: 255, alpha: 255 })
})

test('foreground tile isolation preserves partial-tile gradients and opaque fractional seams with bounded reuse', async ({ page }) => {
  await page.goto('/')
  const evidence = await page.evaluate(async () => {
    const { ViewportTiles } = await import('/src/viewport-tiles.ts' as string); const sampler = new ViewportTiles()
    const source = document.createElement('canvas'); source.width = 2051; source.height = 19; const ctx = source.getContext('2d')!
    const crop = document.createElement('canvas'); crop.width = 2049; crop.height = 17
    const actual = document.createElement('canvas'); actual.width = 997; actual.height = 117; const target = actual.getContext('2d')!
    const reference = document.createElement('canvas'); reference.width = actual.width; reference.height = actual.height
    const view = { x: 1, y: 1, width: 2049, height: 17 }; const destination = { x: 0, y: 0, width: actual.width, height: actual.height }
    ctx.fillStyle = 'red'; ctx.fillRect(0, 0, source.width, source.height); ctx.fillStyle = 'blue'; ctx.fillRect(1, 1, 2049, 17)
    sampler.draw(target, source, view, destination); let solidError = 0
    const solid = target.getImageData(0, 0, actual.width, actual.height).data
    for (let i = 0; i < solid.length; i++) solidError = Math.max(solidError, Math.abs(solid[i] - ([0, 0, 255, 255][i % 4])))
    const pixels = ctx.createImageData(source.width, source.height)
    for (let y = 0; y < source.height; y++) for (let x = 0; x < source.width; x++) { const i = (y * source.width + x) * 4; pixels.data[i] = Math.round(x / (source.width - 1) * 255); pixels.data[i + 1] = Math.round(y / (source.height - 1) * 255); pixels.data[i + 2] = 150; pixels.data[i + 3] = 255 }
    ctx.putImageData(pixels, 0, 0); crop.getContext('2d')!.drawImage(source, 1, 1, 2049, 17, 0, 0, 2049, 17); reference.getContext('2d')!.drawImage(crop, 0, 0, actual.width, actual.height)
    sampler.draw(target, source, view, destination); const a = target.getImageData(0, 0, actual.width, actual.height).data; const b = reference.getContext('2d')!.getImageData(0, 0, actual.width, actual.height).data; let gradientError = 0
    for (let i = 0; i < a.length; i++) gradientError = Math.max(gradientError, Math.abs(a[i] - b[i]))
    ctx.clearRect(0, 0, source.width, source.height); sampler.draw(target, source, view, destination); let stale = 0; const transparent = target.getImageData(0, 0, actual.width, actual.height).data
    for (let i = 0; i < transparent.length; i++) if (i % 4 !== 3) stale = Math.max(stale, transparent[i])
    for (let width = 1010; width < 1018; width++) sampler.draw(target, source, { ...view, width }, destination)
    ctx.fillStyle = 'blue'; ctx.fillRect(1, 1, 2049, 17); target.setTransform(.713, 0, 0, .821, .37, .29)
    const translated = { x: 2.3, y: 1.7, width: 1327.8, height: 120.3 }; sampler.draw(target, source, view, translated)
    const left = Math.round(translated.x * .713 + .37); const top = Math.round(translated.y * .821 + .29); const right = Math.round((translated.x + translated.width) * .713 + .37); const bottom = Math.round((translated.y + translated.height) * .821 + .29)
    const transformedData = target.getImageData(left, top, right - left, bottom - top).data; let transformedError = 0
    for (let i = 0; i < transformedData.length; i++) transformedError = Math.max(transformedError, Math.abs(transformedData[i] - [0, 0, 255, 255][i % 4]))
    const cached = [...(sampler as any).tiles.values()] as HTMLCanvasElement[]; const cachedCount = cached.length; const cacheBytes = cached.reduce((sum, tile) => sum + tile.width * tile.height * 4, 0); sampler.release()
    return { solidError, gradientError, stale, transformedError, cachedCount, cacheBytes, released: cached.every(tile => tile.width === 0 && tile.height === 0) }
  })
  writeEvidence('artifacts/vertical-0.5.0/foreground-tile-evidence.json', evidence)
  expect(evidence.transformedError).toBe(0); expect(evidence.solidError).toBe(0); expect(evidence.gradientError).toBeLessThanOrEqual(2); expect(evidence.stale).toBe(0); expect(evidence.cachedCount).toBeLessThanOrEqual(4); expect(evidence.cacheBytes).toBeLessThanOrEqual(4 * 512 * 512 * 4); expect(evidence.released).toBe(true)
})

test('video background excludes crop and framing content, updates paused masks, and stays solid during original inspection', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/camera-colors.mp4', cameraPath: null, width: 320, height: 180, duration: 1, cursor: [], edits: { trimEnd: 1, backgroundMode: 'videoBlur', backgroundBlur: 24, background: '#202020', aspectRatio: '1:1', zooms: [], padding: 180, cornerRadius: 0, showCursor: false, reframeKeyframes: [{ id: 'right-only', time: 0, x: .75, y: .5, scale: 2 }], redactions: [{ id: 'sensitive', start: 0, end: 1, x: .5, y: 0, width: .5, height: 1, mode: 'blur', strength: 4, tracking: { reviewed: true, keyframes: [{ time: 0, x: .5, y: 0, width: .5, height: 1, confidence: 1, kind: 'manual' }] } }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240); await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(240)
  await page.getByLabel('Crop recording', { exact: true }).check(); await page.getByLabel('Left %', { exact: true }).fill('50'); await page.getByLabel('Left %', { exact: true }).blur(); await expect.poll(async () => (await screenPixel(page, .01, .99))[2]).toBeGreaterThan(240)
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('sensitive'); await page.getByLabel('Effect', { exact: true }).selectOption('solid')
  await expect.poll(() => screenPixel(page, .01, .01)).toEqual([0, 0, 0]); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([0, 0, 0])
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await expect.poll(() => screenPixel(page, .01, .01)).toEqual([32, 32, 32]); await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(240)
  await page.keyboard.press('Escape'); await expect.poll(() => screenPixel(page, .01, .01)).toEqual([0, 0, 0])
  await page.evaluate(() => { (window as any).__holdBackgroundSeek = true; const video = document.querySelector('video[data-track="screen recording"]')!; Object.defineProperty(video, 'seeking', { configurable: true, get: () => (window as any).__holdBackgroundSeek }) })
  await page.getByLabel('Effect', { exact: true }).selectOption('blur'); await expect.poll(() => screenPixel(page, .01, .01)).toEqual([32, 32, 32])
  await page.evaluate(() => { (window as any).__holdBackgroundSeek = false; document.querySelector('video[data-track="screen recording"]')!.dispatchEvent(new Event('seeked')) }); await seekEditor(page, .4); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240)
})

test('video background honors tracked loss and retained frame changes without showing removed red frames', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, cursor: [], duration: 6, edits: { trimEnd: 5, videoClips: [{ id: 'first', start: 0, end: 1 }, { id: 'second', start: 2, end: 3 }, { id: 'last', start: 4, end: 5 }], backgroundMode: 'videoBlur', backgroundBlur: 64, background: '#202020', aspectRatio: '1:1', zooms: [], padding: 180, cornerRadius: 0, showCursor: false, redactions: [{ id: 'lost', start: 4, end: 5, x: .2, y: .2, width: .2, height: .2, mode: 'blur', strength: 4, tracking: { reviewed: false, keyframes: [{ time: 4, x: 0, y: 0, width: 1, height: 1, confidence: 0, kind: 'lost' }] } }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .01, .01))[1]).toBeGreaterThan(100)
  await seekEditor(page, 1.5); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240)
  await seekEditor(page, 2.5); await expect.poll(() => screenPixel(page, .01, .01)).toEqual([0, 0, 0]); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([0, 0, 0])
})

test('paused seeks wait for their own presentation and ignore superseded frame callbacks', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'VideoFrame', { configurable: true, value: undefined })
    const native = HTMLVideoElement.prototype.requestVideoFrameCallback; const w = window as any
    w.__heldPresentations = []; w.__holdPresentation = false
    HTMLVideoElement.prototype.requestVideoFrameCallback = function (callback) { return native.call(this, (now, metadata) => { const deliver = () => callback(now, metadata); if (w.__holdPresentation) w.__heldPresentations.push(deliver); else deliver() }) }
  })
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, systemAudioPath: '/test-media/mix-audio.wav', cursor: [], duration: 6, edits: { systemVolume: 0, trimEnd: 5, videoClips: [{ id: 'a', start: 0, end: 1 }, { id: 'b', start: 2, end: 3 }, { id: 'c', start: 4, end: 5 }], zooms: [], showCursor: false, background: '#202020', backgroundMode: 'videoBlur', padding: 100, cornerRadius: 0, aspectRatio: '1:1' } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const seek = async (value: number) => { await page.getByLabel('Playhead position in seconds').evaluate((element, time) => { (element as HTMLInputElement).value = String(time); element.dispatchEvent(new Event('input', { bubbles: true })) }, value) }
  await page.evaluate(() => { (window as any).__holdPresentation = true }); await seek(1.5)
  await expect.poll(() => page.evaluate(() => (window as any).__heldPresentations.length)).toBeGreaterThan(0)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeVisible(); expect(await screenPixel(page, .5, .5)).toEqual([32, 32, 32])
  await page.evaluate(() => { const w = window as any; w.__stalePresentations = w.__heldPresentations.splice(0) }); await seek(2.5)
  await expect.poll(() => page.evaluate(() => (window as any).__heldPresentations.length)).toBeGreaterThan(0)
  await page.getByRole('slider', { name: 'Computer audio volume' }).press('End')
  await page.evaluate(() => { for (const deliver of (window as any).__stalePresentations) deliver() })
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeVisible(); expect(await screenPixel(page, .5, .5)).toEqual([32, 32, 32])
  await page.evaluate(() => { const w = window as any; w.__holdPresentation = false; for (const deliver of w.__heldPresentations.splice(0)) deliver() })
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden(); await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await expect.poll(() => page.locator('audio[data-track="system audio"]').evaluate(element => element.currentTime)).toBeCloseTo(4.5, 5)
  await seekEditor(page, 1.5); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240)
})

for (const reenable of [false, true]) test(`hiding a decoding webcam preserves the pending screen seek${reenable ? ' and verifies a re-enabled camera' : ' without waiting for the camera'}`, async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    screenPath: '/test-media/cut-zones.mp4', duration: 6, edits: { trimEnd: 6, zooms: [], showCursor: false },
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('tab', { name: 'Camera', exact: true }).click()
  const loading = page.getByText('Seeking edited video...', { exact: true })
  const webcam = page.getByLabel('Show webcam')
  await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await page.evaluate(() => {
    const screen = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!
    const camera = document.querySelector<HTMLVideoElement>('video[data-track="webcam recording"]')!
    const seeking = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'seeking')!.get!
    const time = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'currentTime')!
    const held = new Set([screen, camera])
    for (const media of held) Object.defineProperty(media, 'seeking', { get: () => held.has(media) || seeking.call(media) })
    let cameraSeeks = 0
    Object.defineProperty(camera, 'currentTime', {
      get: () => time.get!.call(camera),
      set: value => { cameraSeeks++; time.set!.call(camera, value) },
    })
    const state = (window as any).__test
    state.cameraSeeks = () => cameraSeeks
    state.releaseDecoder = (track: string) => {
      const media = track === 'screen' ? screen : camera
      held.delete(media)
      media.dispatchEvent(new Event('seeked'))
    }
  })
  await page.getByLabel('Playhead position in seconds').evaluate(element => {
    (element as HTMLInputElement).value = '2.5'
    element.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await expect.poll(() => page.evaluate(() => (window as any).__test.cameraSeeks())).toBe(1)
  await webcam.uncheck()
  if (reenable) await webcam.check()
  await expect(loading).toBeVisible()
  expect(await screenPixel(page, .5, .5)).toEqual([40, 53, 68])
  await page.evaluate(() => (window as any).__test.releaseDecoder('screen'))
  if (reenable) {
    await expect.poll(() => page.evaluate(() => (window as any).__test.cameraSeeks())).toBe(2)
    await expect(loading).toBeVisible()
    await page.evaluate(() => (window as any).__test.releaseDecoder('camera'))
  }
  await expect(loading).toBeHidden({ timeout: 2000 })
  await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(220)
  await expect(page.locator('video[data-track="screen recording"]')).toHaveJSProperty('currentTime', 2.5)
  await expect(page.locator('video[data-track="webcam recording"]')).toHaveJSProperty('paused', true)
  await expect(page.getByRole('alert')).toBeHidden()
})

for (const pauseDuringSeek of [false, true]) test(pauseDuringSeek
  ? 'pausing before the requested frame is presented keeps stale pixels out of the preview'
  : 'a new seek callback carrying the previous frame cannot certify stale pixels', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as any; const NativeFrame = VideoFrame; w.__NativeFrame = NativeFrame; w.__queuedStaleFrames = []
    const request = HTMLVideoElement.prototype.requestVideoFrameCallback
    const cancel = HTMLVideoElement.prototype.cancelVideoFrameCallback
    const waiting = new Map<number, VideoFrameRequestCallback>()
    let presentation: VideoFrameCallbackMetadata | undefined
    HTMLVideoElement.prototype.requestVideoFrameCallback = function (callback) {
      if (this !== w.__staleScreen || !w.__holdStaleFrame) return request.call(this, callback)
      const handle = request.call(this, (now, metadata) => {
        presentation = metadata
        w.__queuedStaleFrames.push(() => {
          if (waiting.delete(handle)) callback(now, { ...metadata, mediaTime: w.__oldFrame.timestamp / 1e6 })
        })
      })
      waiting.set(handle, callback)
      return handle
    }
    HTMLVideoElement.prototype.cancelVideoFrameCallback = function (handle) {
      if (this === w.__staleScreen) waiting.delete(handle)
      cancel.call(this, handle)
    }
    w.__releaseTargetFrame = (target: number) => {
      if (!presentation || !waiting.size) return false
      const frame = new NativeFrame(w.__staleScreen)
      const mediaTime = frame.timestamp / 1e6
      frame.close()
      if (Math.abs(target - mediaTime) > 1 / 30) return false
      w.__holdStaleFrame = false
      for (const [handle, callback] of Array.from(waiting)) {
        if (!waiting.delete(handle)) continue
        cancel.call(w.__staleScreen, handle)
        callback(performance.now(), { ...presentation, mediaTime })
      }
      return true
    }
    w.VideoFrame = new Proxy(NativeFrame, { construct(target, args) { return args[0] === w.__staleScreen && w.__holdStaleFrame ? w.__oldFrame.clone() : Reflect.construct(target, args) } })
    const draw = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) { if (args[0] === w.__staleScreen && w.__holdStaleFrame) args[0] = w.__oldFrame; return (draw as any).apply(this, args) } as typeof draw
  })
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, cursor: [], duration: 6, edits: { trimEnd: 3, videoClips: [{ id: 'a', start: 0, end: 1 }, { id: 'b', start: 2, end: 3 }], zooms: [], showCursor: false, background: '#202020', padding: 100, cornerRadius: 0 } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  const screen = page.locator('video[data-track="screen recording"]')
  if (pauseDuringSeek) {
    await page.getByRole('button', { name: 'Play preview', exact: true }).click()
    await expect.poll(() => screen.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(.05)
  }
  await page.evaluate(() => { const w = window as any; const source = document.querySelector('video[data-track="screen recording"]') as HTMLVideoElement; w.__staleScreen = source; w.__oldFrame = new w.__NativeFrame(source); w.__holdStaleFrame = true; w.__holdStaleDecode = true; const ready = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'readyState')!.get!; Object.defineProperty(source, 'readyState', { configurable: true, get: () => w.__holdStaleDecode ? 1 : ready.call(source) }) })
  try {
    await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '1.5'; element.dispatchEvent(new Event('input', { bubbles: true })) })
    await expect.poll(() => page.evaluate(() => (window as any).__queuedStaleFrames.length)).toBeGreaterThan(0)
    await page.evaluate(() => { const w = window as any; for (const deliver of w.__queuedStaleFrames.splice(0)) deliver(); w.__holdStaleDecode = false; w.__staleScreen.dispatchEvent(new Event('seeked')) })
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
    if (pauseDuringSeek) {
      await expect.poll(() => screen.evaluate(element => !(element as HTMLVideoElement).seeking && (element as HTMLVideoElement).readyState >= 2)).toBe(true)
      await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
      await expect(screen).toHaveJSProperty('paused', true)
    }
    const evidence = await page.evaluate(() => { const w = window as any; const canvas = document.querySelector('canvas.preview-canvas') as HTMLCanvasElement; return { target: w.__staleScreen.currentTime, oldPts: w.__oldFrame.timestamp / 1e6, oldDuration: w.__oldFrame.duration / 1e6, pixel: [...canvas.getContext('2d')!.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data] } })
    if (!pauseDuringSeek) writeEvidence('artifacts/perf-0.5.1/new-seek-stale-frame-evidence.json', evidence)
    await expect(page.getByText('Seeking edited video...', { exact: true })).toBeVisible(); expect(evidence.pixel).toEqual([32, 32, 32, 255])
    await screen.evaluate(element => (element as HTMLVideoElement).requestVideoFrameCallback((_now, metadata) => {
      (window as any).__observedPresentation = metadata.mediaTime
    }))
    await expect.poll(() => page.evaluate(() => (window as any).__releaseTargetFrame(2.5))).toBe(true)
    expect(await page.evaluate(() => (window as any).__observedPresentation)).toBeCloseTo(2.5, 1)
    await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden(); await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(240)
    await expect(screen).toHaveJSProperty('currentTime', 2.5)
    await expect(screen).toHaveJSProperty('paused', true)
  } finally { await page.evaluate(() => { const w = window as any; w.__holdStaleFrame = false; w.__oldFrame?.close() }) }
})

test('playing to the end of a recording leaves its last retained frame visible', async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    screenPath: '/test-media/cut-zones.mp4', cameraPath: null, duration: 6.1,
    edits: { trimEnd: 6.1, zooms: [], showCursor: false },
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await seekEditor(page, 5.7)
  const screen = page.locator('video[data-track="screen recording"]')
  await expect(screen).toHaveJSProperty('currentTime', 5.7)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Pause preview', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeVisible()
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await expect.poll(async () => (await screenPixel(page, .5, .5))[0]).toBeGreaterThan(240)
  expect(await screen.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeCloseTo(6 - 1 / 30, 5)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => screen.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeLessThan(1)
})

test('a loop or trim entirely beyond the decoded screen stops without jumping to another frame', async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    screenPath: '/test-media/cut-zones.mp4', cameraPath: null, duration: 6.14,
    edits: { trimEnd: 6.14, zooms: [], showCursor: false },
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await seekEditor(page, 2.5)
  const screen = page.locator('video[data-track="screen recording"]')
  await expect(screen).toHaveJSProperty('currentTime', 2.5)
  await page.getByRole('button', { name: 'Audition', exact: true }).click()
  const end = page.getByLabel('Loop end (seconds)')
  const start = page.getByLabel('Loop start (seconds)')
  await end.fill('6.14')
  await end.blur()
  await start.fill('6.02')
  await start.blur()
  const loop = page.getByLabel('Loop selection', { exact: true })
  await loop.check()
  const play = page.getByRole('button', { name: 'Play preview', exact: true })
  await play.click()
  await expect(page.getByRole('alert')).toContainText('This loop contains no screen frames')
  await expect(screen).toHaveJSProperty('currentTime', 2.5)
  await expect(screen).toHaveJSProperty('paused', true)
  await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(240)

  await loop.uncheck()
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click()
  const trim = page.locator('#trim-start')
  await trim.fill('6.02')
  await trim.blur()
  await play.click()
  await expect(page.getByRole('alert')).toContainText('This edit contains no screen frames')
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await expect(screen).toHaveJSProperty('currentTime', 2.5)
  await expect(screen).toHaveJSProperty('paused', true)
  expect(await screenPixel(page, .5, .5)).toEqual([40, 53, 68])

  await trim.fill('5.5')
  await trim.blur()
  await expect.poll(async () => (await screenPixel(page, .5, .5))[0]).toBeGreaterThan(240)
  await expect(screen).toHaveJSProperty('currentTime', 5.5)
})

test('video background controls clamp strength, undo and roundtrip presets without storing framing keys', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { reframeKeyframes: [{ id: 'preserved', time: 2, x: .5, y: .5, scale: 1 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); const mode = page.getByLabel('Background style', { exact: true })
  await expect(mode).toHaveValue('solid'); await mode.selectOption('videoBlur'); const strength = page.getByLabel('Background blur strength', { exact: true }); await expect(strength).toHaveValue('24')
  await strength.fill('100'); await strength.blur(); await expect(strength).toHaveValue('64'); await strength.fill('0'); await strength.blur(); await expect(strength).toHaveValue('4')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(strength).toHaveValue('64'); await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(strength).toHaveValue('24'); await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(mode).toHaveValue('solid')
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(mode).toHaveValue('videoBlur'); await page.getByRole('button', { name: 'Save current...' }).click(); await page.getByLabel('Preset name').fill('Video blur test'); await page.getByRole('button', { name: 'Save preset', exact: true }).click()
  const saved = await page.evaluate(() => (window as any).__test.calls.findLast((call: any) => call.name === 'save_appearance_preset').args.settings); expect(saved).toMatchObject({ backgroundMode: 'videoBlur', backgroundBlur: 24 }); expect(saved).not.toHaveProperty('reframeKeyframes')
  await mode.selectOption('solid'); await page.getByRole('button', { name: 'Apply', exact: true }).click(); await expect(mode).toHaveValue('videoBlur'); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.backgroundMode)).toBe('videoBlur'); expect(await page.evaluate(() => (window as any).__test.saved.edits.reframeKeyframes[0].id)).toBe('preserved')
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(mode).toHaveValue('videoBlur'); await expect(strength).toHaveValue('24'); await page.getByLabel('Aspect ratio').selectOption('9:16'); await page.getByRole('button', { name: 'Projects', exact: true }).click(); await page.setViewportSize({ width: 900, height: 640 })
  for (const theme of ['dark', 'light']) { await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme); await strength.focus(); await expect(strength).toBeInViewport(); await page.screenshot({ path: `artifacts/vertical-0.5.0/stage3-${theme}-900x640.png` }) }
})

test('video background excludes camera overlays and remains solid in camera-only gaps', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ screenPath: '/test-media/camera-short.mp4', cameraPath: '/test-media/cut-zones.mp4', cameraIntervals: [{ start: 0, end: .2 }], width: 160, height: 160, duration: 1, cursor: [], edits: { trimEnd: 1, backgroundMode: 'videoBlur', background: '#202020', aspectRatio: '1:1', zooms: [], padding: 180, cornerRadius: 0, showCursor: false, camera: { enabled: true, layout: 'bubble', x: .7, y: .7, width: .2, shape: 'rectangle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240); await expect.poll(async () => (await screenPixel(page, .8, .8))[1]).toBeGreaterThan(100)
  await page.getByRole('tab', { name: 'Camera', exact: true }).click(); await page.getByLabel('Camera layout').selectOption('fullscreen'); await expect.poll(() => screenPixel(page, .01, .01)).toEqual([32, 32, 32]); await seekEditor(page, .5); await expect.poll(() => screenPixel(page, .5, .5)).toEqual([32, 32, 32])
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await expect(page.getByLabel('Background style', { exact: true })).toBeDisabled(); await expect(page.getByLabel('Background blur strength')).toBeDisabled(); await page.getByRole('button', { name: 'Switch to bubble layout' }).click(); await expect.poll(async () => (await screenPixel(page, .01, .01))[2]).toBeGreaterThan(240)
})

test('countdown is cancellable and capture starts only after its chosen deadline', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.clock.install({ time: new Date('2026-11-11T00:00:00Z') }); await page.clock.pauseAt(new Date('2026-11-11T00:00:01Z'))
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(page.getByLabel('Countdown', { exact: true })).toHaveValue('3')
  await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('region', { name: 'Recording countdown' })).toBeVisible()
  await page.clock.runFor(2999); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.getByRole('button', { name: 'Cancel countdown', exact: true }).click(); await page.clock.runFor(5000); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.getByLabel('Countdown', { exact: true }).selectOption('5'); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('region', { name: 'Recording countdown' })).toBeVisible(); await page.keyboard.press('Escape'); await page.clock.runFor(5000)
  await expect(page.getByRole('dialog', { name: 'New recording' })).toBeHidden(); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(page.getByLabel('Countdown', { exact: true })).toHaveValue('5'); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('region', { name: 'Recording countdown' })).toBeVisible()
  await page.clock.runFor(4999); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.clock.runFor(1); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_recording').length)).toBe(1)
})

test('recording dialog keeps actions visible at short heights in both themes, including long failures', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  await page.locator('#webcam').selectOption('cam-test'); await page.getByLabel('Countdown', { exact: true }).selectOption('0')
  const dialog = page.getByRole('dialog', { name: 'New recording' })
  const action = dialog.getByRole('button', { name: /^(Start|Retry) recording$/ })
  const assertBounds = async () => {
    const layout = await dialog.evaluate(element => {
      const body = element.querySelector('.record-dialog-body')!; const footer = element.querySelector('.record-dialog-footer')!; const button = element.querySelector<HTMLButtonElement>('.record-actions button')!; const box = button.getBoundingClientRect(); const bounds = element.getBoundingClientRect()
      return { top: bounds.top, bottom: bounds.bottom, height: innerHeight, overflow: element.scrollWidth - element.clientWidth, scroll: body.scrollHeight > body.clientHeight, buttonTop: box.top, buttonBottom: box.bottom, footerTop: footer.getBoundingClientRect().top, bodyBottom: body.getBoundingClientRect().bottom, hit: document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2) === button }
    })
    expect(layout.top).toBeGreaterThanOrEqual(0); expect(layout.bottom).toBeLessThanOrEqual(layout.height); expect(layout.overflow).toBeLessThanOrEqual(1); expect(layout.buttonBottom).toBeLessThan(layout.height - 8); expect(layout.buttonTop).toBeGreaterThanOrEqual(layout.footerTop); expect(layout.bodyBottom).toBeLessThanOrEqual(layout.footerTop + 1); expect(layout.hit).toBe(true)
  }
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme)
    for (const size of [{ width: 1280, height: 720 }, { width: 900, height: 640 }, { width: 900, height: 480 }]) {
      await page.setViewportSize(size); await assertBounds()
      await dialog.locator('.record-dialog-body').evaluate(element => { element.scrollTop = element.scrollHeight }); await assertBounds()
      await dialog.getByRole('button', { name: 'Enable preview', exact: true }).focus(); await page.keyboard.press('Tab'); await expect(dialog.getByRole('button', { name: 'Test 5 seconds', exact: true })).toBeFocused(); await page.keyboard.press('Tab'); await expect(action).toBeFocused(); expect(await action.evaluate(button => getComputedStyle(button).outlineStyle)).toBe('solid')
      await dialog.locator('.record-dialog-body').evaluate(element => { element.scrollTop = 0 })
      if (size.height === 640) await page.screenshot({ path: path.join(screenshotDir, `record-dialog-041-${theme}-900x640.png`) })
    }
  }
  await page.evaluate(() => { (window as any).__test.recordingStartError = 'The selected display did not produce a video frame. Choose another display or refresh devices. Diagnostic: ' + 'Capture startup returned no frame. '.repeat(24) })
  await action.click(); await expect(dialog.getByRole('alert')).toContainText('Recording did not start'); await assertBounds()
  await expect(page.getByRole('alert')).toHaveCount(1); await expect(dialog.getByRole('alert')).toBeFocused(); await expect(action).toHaveText('Retry recording')
  await page.setViewportSize({ width: 900, height: 640 }); await page.screenshot({ path: path.join(screenshotDir, 'record-dialog-041-failure-light-900x640.png') })
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark' }); await page.screenshot({ path: path.join(screenshotDir, 'record-dialog-041-failure-dark-900x640.png') })
})

test('recording dialog preserves failed options, retries a newly selected source, and never duplicates errors', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: 'New recording' })
  await page.locator('#webcam').selectOption('cam-test'); await page.locator('#microphone').selectOption('mic-test'); await page.getByLabel('Countdown', { exact: true }).selectOption('0')
  await expect(dialog).toContainText('This camera will be recorded. Enable preview to check framing before you start.')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_camera_preview'))).toBe(false)
  await page.evaluate(() => { (window as any).__test.recordingStartError = 'The selected display did not produce a video frame. Choose another display or refresh devices.' })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveCount(1); await expect(dialog.getByRole('alert')).toContainText('selected display'); await expect(dialog).not.toContainText('Restore the window')
  await expect(page.locator('#webcam')).toHaveValue('cam-test'); await expect(page.locator('#microphone')).toHaveValue('mic-test'); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toHaveCount(0)
  await dialog.getByRole('button', { name: 'Window', exact: true }).click(); await expect(dialog).toContainText('Keep this window open and restored while recording.')
  await page.evaluate(() => { (window as any).__test.recordingStartError = null })
  await page.getByRole('button', { name: 'Retry recording', exact: true }).click(); await expect(dialog).toBeHidden(); await expect(page.getByRole('alert')).toHaveCount(0)
  const attempts = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_recording').map((call: any) => call.args.options))
  expect(attempts.map((item: any) => item.sourceId)).toEqual(['screen-test', 'window-test']); expect(attempts[1]).toMatchObject({ cameraId: 'cam-test', microphoneId: 'mic-test' })
})

test('recording dialog rejects stale devices after a failed refresh and ignores replies from a closed setup', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: 'New recording' }); const start = dialog.getByRole('button', { name: 'Start recording', exact: true })
  await expect(start).toBeEnabled(); await page.evaluate(() => { (window as any).__test.devicesError = 'Device enumeration failed for test' })
  await page.getByRole('button', { name: 'Refresh devices', exact: true }).click(); await expect(dialog.getByRole('alert')).toContainText('Device enumeration failed'); await expect(start).toBeDisabled(); await expect(page.getByRole('radio')).toHaveCount(0)
  await page.evaluate(() => { const state = (window as any).__test; state.devicesError = null; state.holdDevices = true })
  await page.getByRole('button', { name: 'Refresh devices', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishDevices)).toBe('function')
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click(); await expect(dialog).toBeHidden()
  await page.evaluate(() => { const state = (window as any).__test; state.holdDevices = false; state.devices = { sources: [{ id: 'new-display', title: 'New display', kind: 'monitor', width: 2560, height: 1440 }], cameras: [], microphones: [] } })
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(page.getByRole('radio', { name: /New display/ })).toBeChecked()
  await page.evaluate(() => (window as any).__test.finishDevices()); await expect(page.getByRole('radio', { name: /New display/ })).toBeChecked(); await expect(page.getByRole('radio', { name: /Test display/ })).toHaveCount(0); await expect(start).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.devices = { sources: [], microphones: [], cameras: [] } }); await page.getByRole('button', { name: 'Refresh devices', exact: true }).click(); await expect(start).toBeDisabled(); await expect(dialog).toContainText('No screens available. Connect or enable a display, then refresh devices.')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
})

const appCaptureCatalog = () => ({
  sources: [{ id: 'screen-test', title: 'Test display', kind: 'monitor', width: 1920, height: 1080 }, { id: 'window-test', title: 'Test window', kind: 'window', width: 960, height: 540 }], microphones: [], cameras: [],
  apps: [
    { id: 'process:41', name: 'Editor.exe', windows: [{ id: 'app:41:101', title: 'Draft chapter', kind: 'window', width: 1280, height: 720 }, { id: 'app:41:102', title: 'Release notes', kind: 'window', width: 1440, height: 900 }] },
    { id: 'process:52', name: 'Browser.exe', windows: [{ id: 'app:52:201', title: 'Product walkthrough', kind: 'window', width: 1920, height: 1080 }] },
  ],
})

async function openAppsPicker(page: Page) {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(devices => { (window as any).__test.devices = devices }, appCaptureCatalog())
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByRole('button', { name: 'Apps', exact: true }).click()
  return page.getByRole('dialog', { name: 'New recording' })
}

test('Apps requires an explicit window and submits its canonical source with honest audio scope', async ({ page }) => {
  const dialog = await openAppsPicker(page); const start = dialog.getByRole('button', { name: 'Start recording', exact: true })
  await expect(dialog.getByRole('radio', { name: 'Editor.exe', exact: true })).toBeChecked(); await expect(dialog.getByRole('radio', { name: 'Editor.exe', exact: true })).toHaveAccessibleDescription('2 windows / Draft chapter'); await expect(start).toBeDisabled()
  await expect(dialog).toContainText('Records one selected window. Other windows and new popups are not included.'); await expect(dialog).toContainText('Computer audio includes sound from all apps.')
  await dialog.getByLabel('Window from Editor.exe', { exact: true }).selectOption('app:41:102'); await expect(start).toBeEnabled(); await expect(dialog.locator('.record-source-name')).toHaveText('Editor.exe: Release notes')
  await dialog.getByRole('button', { name: 'Screen', exact: true }).click(); await expect(dialog.getByRole('radio', { name: /Test display/ })).toBeChecked(); await expect(dialog.locator('.record-source-name')).toHaveText('Test display')
  await dialog.getByRole('button', { name: 'Window', exact: true }).click(); await expect(dialog.getByRole('radio', { name: /Test window/ })).toBeChecked(); await dialog.getByRole('button', { name: 'Apps', exact: true }).click(); await expect(dialog.getByLabel('Window from Editor.exe', { exact: true })).toHaveValue('app:41:102')
  await dialog.getByLabel('Find an app or window').fill('walkthrough'); await expect(dialog.getByRole('radio')).toHaveCount(1); await dialog.getByRole('radio', { name: 'Browser.exe', exact: true }).focus(); await page.keyboard.press('Space'); await expect(dialog.getByLabel('Window from Browser.exe', { exact: true })).toHaveValue('app:52:201')
  await dialog.getByLabel('Find an app or window').fill('missing'); await expect(dialog.getByRole('status').filter({ hasText: 'No apps match this search.' })).toBeVisible(); await expect(dialog.locator('.record-source-name')).toHaveText('Browser.exe: Product walkthrough')
  await dialog.getByLabel('Countdown', { exact: true }).selectOption('0'); await start.click(); await expect(dialog).toBeHidden()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_recording').map((call: any) => call.args.options))).toEqual([{ sourceId: 'app:52:201', fps: 30, microphoneId: null, cameraId: null, systemAudio: true, systemAudioScope: 'system', region: null }])
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('screenrecord-recording-options') ?? '{}').sourceId)).toBeUndefined()
})

test('Apps search Enter never submits recording for the previously selected filtered-out window', async ({ page }) => {
  const dialog = await openAppsPicker(page); await dialog.getByLabel('Window from Editor.exe').selectOption('app:41:102'); await dialog.getByLabel('Countdown', { exact: true }).selectOption('0')
  await dialog.locator('form').evaluate(form => { const state = (window as any).__test; state.searchSubmits = 0; form.addEventListener('submit', () => { state.searchSubmits++ }) })
  const search = dialog.getByLabel('Find an app or window'); await search.fill('walkthrough'); await search.press('Enter')
  expect(await page.evaluate(() => (window as any).__test.searchSubmits)).toBe(0); await expect(dialog).toBeVisible(); await expect(search).toBeFocused(); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await dialog.getByRole('radio', { name: 'Browser.exe', exact: true }).check(); await dialog.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(dialog).toBeHidden()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_recording').map((call: any) => call.args.options.sourceId))).toEqual(['app:52:201'])
})

test('Apps refresh preserves valid selection and requires reselection after app or window disappearance', async ({ page }) => {
  const dialog = await openAppsPicker(page); const start = dialog.getByRole('button', { name: 'Start recording', exact: true }); const refresh = dialog.getByRole('button', { name: 'Refresh devices', exact: true })
  await dialog.getByLabel('Window from Editor.exe').selectOption('app:41:102'); await refresh.click(); await expect(dialog.getByLabel('Window from Editor.exe')).toHaveValue('app:41:102')
  await page.evaluate(() => { (window as any).__test.devices.apps[0].windows.pop() }); await refresh.click(); await expect(start).toBeDisabled(); await expect(dialog.getByLabel('Window from Editor.exe')).toHaveValue(''); await expect(dialog).toContainText('The selected app or window is no longer available.')
  await refresh.click(); await expect(start).toBeDisabled(); await start.evaluate(button => (button as HTMLButtonElement).click()); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await dialog.getByLabel('Window from Editor.exe').selectOption('app:41:101'); await expect(start).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.devices.apps.shift() }); await refresh.click(); await expect(start).toBeDisabled(); await expect(dialog.getByRole('radio', { name: 'Browser.exe', exact: true })).not.toBeChecked()
  await dialog.getByRole('button', { name: 'Screen', exact: true }).click(); await dialog.getByRole('button', { name: 'Apps', exact: true }).click(); await expect(start).toBeDisabled()
  await page.evaluate(devices => { (window as any).__test.devices = devices }, appCaptureCatalog()); await refresh.click(); await expect(start).toBeDisabled(); await expect(dialog.getByRole('radio', { name: 'Editor.exe', exact: true })).not.toBeChecked()
  await dialog.getByRole('radio', { name: 'Browser.exe', exact: true }).check(); await expect(start).toBeEnabled(); await expect(dialog.locator('.record-source-name')).toHaveText('Browser.exe: Product walkthrough')
})

test('Apps handles legacy empty catalogs and rejects late enumeration from a closed setup', async ({ page }) => {
  const dialog = await openAppsPicker(page); const refresh = dialog.getByRole('button', { name: 'Refresh devices', exact: true }); const start = dialog.getByRole('button', { name: 'Start recording', exact: true })
  await dialog.getByLabel('Window from Editor.exe').selectOption('app:41:102'); await page.evaluate(() => { const state = (window as any).__test; state.devicesError = 'Apps enumeration failed' }); await refresh.click(); await expect(start).toBeDisabled(); await expect(dialog.getByRole('alert')).toContainText('Apps enumeration failed'); await expect(dialog.getByRole('radio')).toHaveCount(0)
  await page.evaluate(() => { const state = (window as any).__test; state.devicesError = null; state.holdDevices = true }); await refresh.click(); await expect(dialog.getByRole('status').filter({ hasText: 'Finding screens and devices...' })).toBeVisible(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishDevices)).toBe('function')
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click(); await page.evaluate(() => { const state = (window as any).__test; state.holdDevices = false; state.devices.apps = state.devices.apps.slice(1) }); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(dialog.getByRole('radio', { name: 'Browser.exe', exact: true })).toBeVisible(); await expect(start).toBeDisabled()
  await page.evaluate(() => (window as any).__test.finishDevices()); await expect(dialog.getByRole('radio', { name: 'Editor.exe', exact: true })).toHaveCount(0); await expect(start).toBeDisabled()
  await page.evaluate(() => { delete (window as any).__test.devices.apps }); await refresh.click(); await expect(dialog).toContainText('No apps with available windows.'); await expect(dialog.getByRole('radio')).toHaveCount(0); await expect(start).toBeDisabled()
  await dialog.getByRole('button', { name: 'Window', exact: true }).click(); await expect(dialog.getByRole('radio', { name: /Test window/ })).toBeChecked(); await expect(start).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
})

test('Apps keeps keyboard focus, readable choices and recording actions within a short viewport in both themes', async ({ page }) => {
  const dialog = await openAppsPicker(page); await dialog.getByLabel('Window from Editor.exe').selectOption('app:41:102'); await page.setViewportSize({ width: 900, height: 640 }); mkdirSync('artifacts/apps-0.4.4', { recursive: true })
  const windowChoice = dialog.getByLabel('Window from Editor.exe'); const start = dialog.getByRole('button', { name: 'Start recording', exact: true })
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme); await windowChoice.focus(); expect(await windowChoice.evaluate(element => getComputedStyle(element).outlineStyle)).toBe('solid')
    const bounds = await dialog.evaluate(element => { const button = element.querySelector('.record-actions button')!; const box = button.getBoundingClientRect(); return { overflow: element.scrollWidth - element.clientWidth, bottom: box.bottom, visible: document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2) === button } })
    expect(bounds.overflow).toBeLessThanOrEqual(1); expect(bounds.bottom).toBeLessThan(640); expect(bounds.visible).toBe(true); await expect(start).toBeEnabled()
    await dialog.locator('.record-dialog-body').evaluate(element => { element.scrollTop = 0 }); await page.screenshot({ path: `artifacts/apps-0.4.4/apps-${theme}-900x640.png` })
    await dialog.getByText('Computer audio includes sound from all apps.', { exact: true }).scrollIntoViewIfNeeded(); await expect(dialog.getByText('Computer audio includes sound from all apps.', { exact: true })).toBeInViewport()
  }
})

test('five-second recording test is opt-in, stops previews, reviews isolated tracks and cleans up before Start', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 640 }); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click()
  const setup = page.getByRole('dialog', { name: 'New recording' }); await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => ['run_recording_test', 'get_source_thumbnail', 'start_recording'].includes(c.name)))).toBe(false)
  await setup.getByLabel('Webcam', { exact: true }).selectOption('cam-test'); await setup.getByLabel('Microphone', { exact: true }).selectOption('mic-test')
  await setup.getByRole('button', { name: 'Enable preview' }).click(); await setup.getByRole('button', { name: 'Test microphone' }).click()
  await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Listen to')).toBeVisible(); await expect(setup.getByLabel('Listen to')).toHaveValue('none')
  await expect.poll(() => setup.getByLabel('Recorded test screen').evaluate((video: HTMLVideoElement) => video.readyState)).toBe(4)
  for (const theme of ['dark', 'light']) { await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme); await setup.locator('.record-dialog-body').evaluate(element => { element.scrollTop = element.scrollHeight }); await page.screenshot({ path: path.resolve(`artifacts/preflight-0.6.0/trial-${theme}-900.png`) }) }
  const order = await page.evaluate(() => (window as any).__test.calls.map((c: any) => c.name)); expect(order.lastIndexOf('stop_camera_preview')).toBeLessThan(order.indexOf('run_recording_test')); expect(order.lastIndexOf('stop_microphone_meter')).toBeLessThan(order.indexOf('run_recording_test'))
  expect(await setup.locator('audio').evaluateAll(tracks => tracks.every(track => (track as HTMLMediaElement).muted))).toBe(true)
  await setup.getByLabel('Listen to').selectOption('microphone'); expect(await setup.locator('audio').evaluateAll(tracks => tracks.map(track => (track as HTMLMediaElement).muted))).toEqual([false, true])
  await setup.getByLabel('Recorded test screen').evaluate((video: HTMLVideoElement) => video.play())
  await expect.poll(() => setup.locator('audio').first().evaluate((audio: HTMLAudioElement) => !audio.paused && !audio.muted && audio.volume === 1 && audio.currentTime > .4)).toBe(true)
  await expect.poll(() => setup.getByLabel('Recorded test webcam').evaluate((video: HTMLVideoElement) => !video.paused && video.currentTime > .4)).toBe(true)
  await setup.getByLabel('Listen to').selectOption('system'); expect(await setup.locator('audio').evaluateAll(tracks => tracks.map(track => (track as HTMLMediaElement).muted))).toEqual([true, false])
  await expect.poll(() => setup.locator('audio').evaluateAll(tracks => (tracks[0] as HTMLAudioElement).paused && !(tracks[1] as HTMLAudioElement).paused && (tracks[1] as HTMLAudioElement).currentTime > .4)).toBe(true)
  await setup.getByLabel('Recorded test screen').evaluate(async (video: HTMLVideoElement) => { video.currentTime = 1; await new Promise<void>(resolve => video.addEventListener('seeked', () => resolve(), { once: true })) })
  await expect.poll(() => setup.locator('.trial-videos').evaluate(element => { const media = element.querySelectorAll('video'); return media[1].currentTime - media[0].currentTime })).toBeCloseTo(.2, 2)
  expect(await setup.locator('audio').evaluateAll(tracks => tracks.every(track => (track as HTMLMediaElement).paused))).toBe(true)
  await setup.getByLabel('Recorded test screen').evaluate((video: HTMLVideoElement) => video.play()); await expect.poll(() => setup.locator('audio').nth(1).evaluate((audio: HTMLAudioElement) => !audio.paused && audio.currentTime > 1.3)).toBe(true)
  await page.evaluate(() => { const original = (window as any).__TAURI_INTERNALS__.invoke; (window as any).__TAURI_INTERNALS__.invoke = (name: string, args: any) => { if (name === 'discard_recording_test') (window as any).__test.releaseBeforeDiscard = [...document.querySelectorAll('.recording-trial video, .recording-trial audio')].every((element: any) => element.paused && !element.hasAttribute('src') && element.readyState === 0); return original(name, args) } })
  await setup.getByLabel('Countdown').selectOption('0'); await setup.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'start_recording'))).toBe(true)
  const calls = await page.evaluate(() => (window as any).__test.calls); expect(await page.evaluate(() => (window as any).__test.releaseBeforeDiscard)).toBe(true); expect(calls.findIndex((c: any) => c.name === 'discard_recording_test')).toBeLessThan(calls.findIndex((c: any) => c.name === 'start_recording')); expect(calls.find((c: any) => c.name === 'run_recording_test').args.options).toEqual(calls.find((c: any) => c.name === 'start_recording').args.options)
})

test('recording test cancellation waits for late completion, ignores stale phases and keeps cleanup errors retryable', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click()
  await page.evaluate(() => { (window as any).__test.holdTrial = true; (window as any).__test.lateTrial = true })
  const setup = page.getByRole('dialog', { name: 'New recording' }); await setup.getByRole('button', { name: 'Test 5 seconds' }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishTrial)).toBe('function')
  const id = await page.evaluate(() => (window as any).__test.calls.find((c: any) => c.name === 'run_recording_test').args.id)
  await page.evaluate(id => (window as any).__test.emit('recording-test-phase', { id, phase: 'finalizing' }), id); await expect(setup.getByRole('region', { name: 'Recording test', exact: true }).getByRole('status')).toContainText('Finalizing test')
  await setup.getByRole('button', { name: 'Cancel test' }).click(); await expect(setup.getByRole('button', { name: 'Start recording', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.finishTrial()); await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled(); await expect(setup.getByLabel('Recorded test screen')).toHaveCount(0)
  await page.evaluate(id => (window as any).__test.emit('recording-test-phase', { id, phase: 'recording' }), id); await expect(setup.getByText('Recording five seconds...', { exact: false })).toHaveCount(0)
  await page.evaluate(() => { (window as any).__test.holdTrial = false; (window as any).__test.discardTrialError = 'Files are still in use' })
  await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Listen to')).toBeVisible(); await setup.getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect(setup).toBeVisible(); await expect(setup.getByText('Files are still in use', { exact: false }).first()).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'start_recording'))).toBe(false)
  await page.evaluate(() => { (window as any).__test.discardTrialError = null }); await setup.getByRole('button', { name: 'Retry discard' }).click(); await setup.getByRole('button', { name: 'Cancel', exact: true }).click(); await expect(setup).not.toBeVisible()
})

test('recording tests reject wrong IDs, invalidate on settings edits and cancel during preparation before any capture', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await page.evaluate(() => { (window as any).__test.trialWrongId = true }); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByRole('alert')).toContainText('valid result'); await expect(setup.getByLabel('Recorded test screen')).toHaveCount(0)
  await page.evaluate(() => { (window as any).__test.trialWrongId = false }); await setup.getByRole('button', { name: 'Retry test' }).click(); await expect(setup.getByLabel('Listen to')).toBeVisible()
  await setup.getByLabel('Frame rate').selectOption('60'); await expect(setup.getByLabel('Recorded test screen')).toHaveCount(0); await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled()
  await setup.getByLabel('Webcam', { exact: true }).selectOption('cam-test'); await setup.getByRole('button', { name: 'Enable preview' }).click(); await page.evaluate(() => { (window as any).__test.holdPreviewStop = true })
  await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishPreviewStop)).toBe('function'); await setup.getByRole('button', { name: 'Cancel test' }).click()
  await page.evaluate(() => { (window as any).__test.holdPreviewStop = false; (window as any).__test.finishPreviewStop() }); await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((c: any) => c.name === 'run_recording_test').length)).toBe(2)
})

test('a fresh full source accepts the first draw in either direction and ignores secondary pointers', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  for (const reverse of [false, true]) {
    await setup.getByRole('button', { name: 'Choose area' }).click(); const frame = page.getByRole('dialog', { name: 'Capture area', exact: true }); const surface = frame.getByRole('group', { name: 'Selected capture frame' }); await expect(surface).toBeVisible()
    const box = await surface.boundingBox(); if (!box) throw new Error('Missing source frame'); const a = reverse ? .75 : .25; const b = reverse ? .25 : .75
    await page.mouse.move(box.x + box.width * a, box.y + box.height * a); await page.mouse.down()
    await surface.dispatchEvent('pointerdown', { pointerId: 22, pointerType: 'touch', button: 0, clientX: box.x + 4, clientY: box.y + 4 }); await surface.dispatchEvent('pointerup', { pointerId: 22, pointerType: 'touch', button: 0 })
    await page.mouse.move(box.x + box.width * b, box.y + box.height * b); await page.mouse.up()
    await expect(frame.getByLabel('Frame x', { exact: true })).toHaveValue('480'); await expect(frame.getByLabel('Frame y', { exact: true })).toHaveValue('270'); await expect(frame.getByLabel('Frame width', { exact: true })).toHaveValue('960'); await expect(frame.getByLabel('Frame height', { exact: true })).toHaveValue('540')
    await frame.getByRole('button', { name: 'Use this area' }).click(); await expect(setup.getByText('960 x 540 at 480, 270')).toBeVisible()
    if (!reverse) await setup.getByRole('button', { name: 'Full source', exact: true }).click()
  }
  await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Listen to')).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.calls.find((c: any) => c.name === 'run_recording_test').args.options.region)).toEqual({ x: 480, y: 270, width: 960, height: 540 })
})

test('source frame draws, resizes and moves in source pixels, resets on source switch and is identical for test and recording', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await setup.getByRole('button', { name: 'Choose area' }).click(); const frame = page.getByRole('dialog', { name: 'Capture area', exact: true }); const surface = frame.getByRole('group', { name: 'Selected capture frame' }); await expect(surface).toBeVisible()
  await frame.getByLabel('Frame x', { exact: true }).fill('100'); await frame.getByLabel('Frame width', { exact: true }).fill('801'); await frame.getByLabel('Frame height', { exact: true }).fill('401'); await frame.getByLabel('Frame height', { exact: true }).press('Tab')
  await expect(frame.getByLabel('Frame width', { exact: true })).toHaveValue('800'); await expect(frame.getByLabel('Frame height', { exact: true })).toHaveValue('400')
  await surface.focus(); await page.keyboard.press('ArrowRight'); await page.keyboard.press('Shift+ArrowDown'); await expect(frame.getByLabel('Frame x', { exact: true })).toHaveValue('1'); await expect(frame.getByLabel('Frame height', { exact: true })).toHaveValue('402')
  const box = await surface.boundingBox(); if (!box) throw new Error('Missing source frame'); await page.mouse.move(box.x + box.width * .8, box.y + box.height * .8); await page.mouse.down(); await page.mouse.move(box.x + box.width * .5, box.y + box.height * .5); await page.mouse.up()
  await expect(frame.getByLabel('Frame x', { exact: true })).toHaveValue('960'); await expect(frame.getByLabel('Frame y', { exact: true })).toHaveValue('540')
  await frame.getByRole('button', { name: 'Use this area' }).click(); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Listen to')).toBeVisible()
  await setup.getByLabel('Countdown').selectOption('0'); await setup.getByRole('button', { name: 'Start recording', exact: true }).click(); const calls = await page.evaluate(() => (window as any).__test.calls); const region = calls.find((c: any) => c.name === 'run_recording_test').args.options.region
  expect(region).toMatchObject({ x: 960, y: 540 }); expect(region.width % 2).toBe(0); expect(region.height % 2).toBe(0); expect(calls.find((c: any) => c.name === 'start_recording').args.options.region).toEqual(region)
})

test('recording source changes retain keyboard focus and area close restores its explicit trigger', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled(); await expect(setup.getByRole('button', { name: 'Choose area', exact: true })).not.toBeFocused()
  await setup.getByRole('button', { name: 'Window', exact: true }).click(); await expect(setup.getByRole('button', { name: 'Window', exact: true })).toBeFocused()
  await setup.getByRole('button', { name: 'Choose area' }).click(); const frame = page.getByRole('dialog', { name: 'Capture area', exact: true }); await expect(frame.getByRole('group', { name: 'Selected capture frame' })).toBeVisible(); await frame.getByRole('button', { name: 'Cancel', exact: true }).click(); await expect(setup.getByRole('button', { name: 'Choose area' })).toBeFocused()
  await setup.getByRole('button', { name: 'Screen', exact: true }).click(); await expect(setup.getByRole('button', { name: 'Screen', exact: true })).toBeFocused()
})

test('source thumbnail stale completion cannot apply to another source and failed requests can retry without recording', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await page.evaluate(() => { (window as any).__test.holdThumbnail = true }); await setup.getByRole('button', { name: 'Choose area' }).click(); const frame = page.getByRole('dialog', { name: 'Capture area', exact: true }); await expect(frame.getByRole('status')).toContainText('Loading')
  await frame.getByRole('button', { name: 'Cancel', exact: true }).click(); await setup.getByRole('button', { name: 'Window', exact: true }).click(); await page.evaluate(() => { (window as any).__test.holdThumbnail = false; (window as any).__test.finishThumbnail(); (window as any).__test.thumbnailError = 'Window is minimized' })
  await setup.getByRole('button', { name: 'Choose area' }).click(); await expect(frame.getByRole('alert')).toContainText('Window is minimized'); await page.evaluate(() => { (window as any).__test.thumbnailError = null }); await frame.getByRole('button', { name: 'Retry preview' }).click(); await expect(frame.getByRole('group', { name: 'Selected capture frame' })).toBeVisible()
  await frame.getByRole('button', { name: 'Use full source' }).click(); expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => ['start_recording', 'run_recording_test'].includes(c.name)))).toBe(false)
})

test('Apps audio scope stays explicit through test errors and resets only when leaving Apps', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { (window as any).__test.devices = { sources: [{ id: 'screen-test', title: 'Test display', kind: 'monitor', width: 1920, height: 1080 }], cameras: [], microphones: [], apps: [{ id: 'process:42', name: 'Game.exe', windows: [{ id: 'app:42:101', title: 'Test game', kind: 'window', width: 1920, height: 1080 }] }] } }); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await setup.getByRole('button', { name: 'Apps', exact: true }).click(); await expect(setup.getByLabel('Computer audio source')).toHaveValue('system'); await setup.getByLabel('Computer audio source').selectOption('application'); await expect(setup.getByText('Other applications are excluded.', { exact: false })).toBeVisible()
  await page.evaluate(() => { (window as any).__test.trialError = 'Application audio is unavailable' }); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByRole('alert')).toContainText('Application audio is unavailable'); await expect(setup.getByLabel('Computer audio source')).toHaveValue('application'); expect(await page.evaluate(() => (window as any).__test.calls.find((c: any) => c.name === 'run_recording_test').args.options.systemAudioScope)).toBe('application')
  await setup.getByRole('button', { name: 'Screen', exact: true }).click(); await setup.getByRole('button', { name: 'Apps', exact: true }).click(); await expect(setup.getByLabel('Computer audio source')).toHaveValue('system')
})

test('Apps audio mute and unmute preserve application scope for test and real recording', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { (window as any).__test.devices = { sources: [], cameras: [], microphones: [], apps: [{ id: 'process:42', name: 'Game.exe', windows: [{ id: 'app:42:101', title: 'Test game', kind: 'window', width: 1920, height: 1080 }] }] } })
  await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await setup.getByRole('button', { name: 'Apps', exact: true }).click(); await setup.getByLabel('Computer audio source').selectOption('application'); await setup.getByLabel('Record computer audio', { exact: true }).uncheck()
  await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Recorded test screen')).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.find((c: any) => c.name === 'run_recording_test').args.options)).toMatchObject({ sourceId: 'app:42:101', systemAudio: false, systemAudioScope: 'system' })
  await setup.getByLabel('Record computer audio', { exact: true }).check(); await expect(setup.getByLabel('Computer audio source')).toHaveValue('application'); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByLabel('Recorded test screen')).toBeVisible()
  await setup.getByLabel('Countdown').selectOption('0'); await setup.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'start_recording'))).toBe(true)
  const calls = await page.evaluate(() => (window as any).__test.calls); expect(calls.filter((c: any) => c.name === 'run_recording_test')[1].args.options).toMatchObject({ sourceId: 'app:42:101', systemAudio: true, systemAudioScope: 'application' }); expect(calls.find((c: any) => c.name === 'start_recording').args.options).toMatchObject({ sourceId: 'app:42:101', systemAudio: true, systemAudioScope: 'application' })
})

test('shortcut cancellation returns the recording test to idle without a failure banner', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await page.evaluate(() => { (window as any).__test.trialError = 'Recording test cancelled.' }); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect(setup.getByText('Test cancelled.', { exact: true })).toBeVisible(); await expect(setup.getByRole('alert')).toHaveCount(0); await expect(setup.getByRole('button', { name: 'Test 5 seconds' })).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((c: any) => c.name === 'discard_recording_test').length)).toBe(1)
})

test('native close drains a pending trial and Escape closes setup without a late result', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await page.evaluate(() => { (window as any).__test.holdTrial = true; (window as any).__test.lateTrial = true }); await setup.getByRole('button', { name: 'Test 5 seconds' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishTrial)).toBe('function')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', {})); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'cancel_recording_test'))).toBe(true); expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(false)
  await page.evaluate(() => (window as any).__test.finishTrial()); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(true)
  await page.keyboard.press('Escape'); await expect(setup).not.toBeVisible(); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); await expect(setup.getByLabel('Recorded test screen')).toHaveCount(0)
})

test('capture area pointer cancellation, reset and short themed dialogs remain keyboard accessible', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 640 }); await page.goto('/'); await page.getByRole('banner').getByRole('button', { name: 'New recording', exact: true }).click(); const setup = page.getByRole('dialog', { name: 'New recording' })
  await setup.getByRole('button', { name: 'Choose area' }).click(); const frame = page.getByRole('dialog', { name: 'Capture area', exact: true }); const surface = frame.getByRole('group', { name: 'Selected capture frame' }); await expect(surface).toBeVisible()
  await frame.getByLabel('Frame width', { exact: true }).fill('800'); await frame.getByLabel('Frame height', { exact: true }).fill('400'); await frame.getByLabel('Frame height', { exact: true }).press('Tab')
  const box = await surface.boundingBox(); if (!box) throw new Error('Missing source frame'); await page.mouse.move(box.x + box.width * .1, box.y + box.height * .1); await page.mouse.down(); await page.mouse.move(box.x + box.width * .3, box.y + box.height * .3); await page.keyboard.press('Escape'); await page.mouse.up(); await expect(frame.getByLabel('Frame x', { exact: true })).toHaveValue('0'); await expect(frame).toBeVisible()
  await frame.getByRole('button', { name: 'Use this area' }).click(); await expect(setup.getByText('800 x 400 at 0, 0')).toBeVisible()
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme)
    await setup.getByRole('button', { name: 'Choose area' }).click(); await expect(surface).toBeVisible(); await frame.getByRole('button', { name: 'Cancel', exact: true }).focus(); await page.keyboard.press('Tab'); await expect(surface).toBeFocused(); expect(await surface.evaluate(element => getComputedStyle(element).outlineStyle)).toBe('solid')
    for (const control of [frame.getByRole('button', { name: 'Use this area' }), frame.getByRole('button', { name: 'Cancel', exact: true })]) { const bounds = await control.boundingBox(); expect(bounds?.y).toBeGreaterThanOrEqual(0); expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(640) }
    await page.screenshot({ path: path.resolve(`artifacts/preflight-0.6.0/frame-${theme}-900.png`) }); await frame.getByRole('button', { name: 'Cancel', exact: true }).click()
    await setup.locator('.record-dialog-body').evaluate(element => { element.scrollTop = element.scrollHeight }); await page.screenshot({ path: path.resolve(`artifacts/preflight-0.6.0/setup-${theme}-900.png`) })
  }
  await setup.getByRole('button', { name: 'Full source', exact: true }).click(); await expect(setup.getByText('Full source', { exact: true })).toBeVisible()
  await setup.getByRole('button', { name: 'Choose area' }).click(); await frame.getByRole('button', { name: 'Use this area' }).click(); await setup.getByRole('button', { name: 'Window', exact: true }).click(); await expect(setup.getByText('Full source', { exact: true })).toBeVisible()
})

test('recording startup freezes its submitted choices and leaves an async failure retryable', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0')
  await page.evaluate(() => { const state = (window as any).__test; state.holdRecordingStart = true; state.recordingStartError = 'The selected display did not produce a video frame.' })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingStart)).toBe('function')
  const dialog = page.getByRole('dialog', { name: 'New recording' }); await expect(dialog.locator('.record-layout')).toHaveAttribute('inert', ''); await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeDisabled(); await expect(page.locator('#webcam')).toBeDisabled()
  await page.keyboard.press('Escape'); await expect(dialog).toBeVisible()
  await page.evaluate(() => { const state = (window as any).__test; state.holdRecordingStart = false; state.finishRecordingStart() }); await expect(page.getByRole('button', { name: 'Retry recording', exact: true })).toBeEnabled(); await expect(dialog.locator('.record-layout')).not.toHaveAttribute('inert', '')
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click(); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(page.getByRole('alert')).toHaveCount(0); await expect(page.getByRole('button', { name: 'Start recording', exact: true })).toBeEnabled()
  const attempts = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_recording')); expect(attempts).toHaveLength(1); expect(attempts[0].args.options.sourceId).toBe('screen-test')
})

test('recording dialog keeps failed camera cleanup retryable and reports preview failures only at the camera', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#webcam').selectOption('cam-test')
  await page.evaluate(() => { (window as any).__test.previewError = 'Camera disconnected for test' }); await page.getByRole('button', { name: 'Enable preview', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveCount(1); await expect(page.getByRole('region', { name: 'Camera preview', exact: true }).getByRole('alert')).toHaveText('Camera disconnected for test')
  await page.evaluate(() => { (window as any).__test.previewError = null }); await page.getByRole('button', { name: 'Retry preview', exact: true }).click(); await expect(page.getByRole('img', { name: 'Live preview of the selected camera' })).toBeVisible()
  await page.evaluate(() => { (window as any).__test.previewStopError = 'Camera cleanup failed for test' }); await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Retry stop preview', exact: true })).toBeVisible(); await expect(page.getByText('Camera cleanup failed for test', { exact: true })).toHaveCount(1)
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  const priorStarts = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_camera_preview').length)
  await page.evaluate(() => { (window as any).__test.previewStopError = null }); await page.getByRole('button', { name: 'Retry stop preview', exact: true }).click(); await expect(page.getByRole('button', { name: 'Enable preview', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_camera_preview').length)).toBe(priorStarts)
  const sessions = await page.evaluate(() => { const calls = (window as any).__test.calls; return { started: calls.filter((call: any) => call.name === 'start_camera_preview').at(-1).args.sessionId, stopped: calls.filter((call: any) => call.name === 'stop_camera_preview').at(-1).args.sessionId } }); expect(sessions.stopped).toBe(sessions.started)
  await page.getByLabel('Countdown', { exact: true }).selectOption('0'); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeVisible(); await expect(page.getByRole('alert')).toHaveCount(0)
})

test('native close cancels a recording countdown before capture can begin', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.clock.install({ time: new Date('2026-11-11T00:00:00Z') }); await page.clock.pauseAt(new Date('2026-11-11T00:00:01Z'))
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('region', { name: 'Recording countdown' })).toBeVisible()
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null)); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(true)
  await page.clock.runFor(5000); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false); await expect(page.getByRole('alert')).toHaveCount(0)
})

test('recovery notices contain meaningful distinct issues and disappear when recovery is clear', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.recoveryIssues = ['', '  ', '\n']; state.emit('recovery-issues-updated', null) }); await expect(page.locator('.recovery-issues')).toHaveCount(0)
  await page.evaluate(() => { const state = (window as any).__test; state.recoveryIssues = ['', 'Recovered screen, camera was unavailable.', ' Recovered screen, camera was unavailable. ']; state.emit('recovery-issues-updated', null) }); await expect(page.locator('.recovery-issues summary')).toHaveText('1 project recovery issue'); await page.locator('.recovery-issues summary').click(); await expect(page.locator('.recovery-issues li')).toHaveText(['Recovered screen, camera was unavailable.'])
  await page.evaluate(() => { const state = (window as any).__test; state.recoveryIssues = []; state.emit('recovery-issues-updated', null) }); await expect(page.locator('.recovery-issues')).toHaveCount(0)
})

test('restored main controls pause and resume the same recording while ignoring an older status poll', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0'); await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeVisible()
  await page.evaluate(() => { (window as any).__test.holdRecordingState = true }); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingState)).toBe('function')
  await page.getByRole('button', { name: 'Pause recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Resume recording', exact: true })).toBeVisible()
  await page.evaluate(() => (window as any).__test.finishRecordingState()); await expect(page.getByText('Recording paused', { exact: true })).toBeVisible()
  await page.evaluate(() => { (window as any).__test.pauseError = 'Resume failed for test' }); await page.getByRole('button', { name: 'Resume recording', exact: true }).click(); await expect(page.getByRole('alert')).toContainText('Resume failed for test'); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.pauseError = null }); await page.getByRole('button', { name: 'Resume recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeVisible(); await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  const calls = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => ['pause_recording', 'resume_recording', 'stop_recording'].includes(call.name)))
  expect(calls.every((call: any) => call.args.projectId === 'ui-test')).toBe(true)
})

test('late completion and error events cannot affect a new recording during startup or while active', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.holdRecordingStart = true; state.recordingProjectId = 'new-session' })
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0'); await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingStart)).toBe('function')
  const emitOldStop = () => page.evaluate(() => { const state = (window as any).__test; state.emit('recording-stopped', state.currentProject()); state.emit('recording-error', { projectId: 'ui-test', error: 'Old session disconnected' }) })
  await emitOldStop(); await expect(page.getByRole('dialog', { name: 'New recording' })).toBeVisible(); await expect(page.getByLabel('Project name')).toHaveCount(0); await expect(page.getByRole('alert')).toHaveCount(0)
  await page.evaluate(() => (window as any).__test.finishRecordingStart()); await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeEnabled()
  await emitOldStop(); await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeEnabled(); await expect(page.getByLabel('Project name')).toHaveCount(0); await expect(page.getByRole('alert')).toHaveCount(0)
  await page.evaluate(() => (window as any).__test.emit('recording-error', { projectId: 'new-session', error: 'Current session disconnected' })); await expect(page.getByRole('alert')).toContainText('Current session disconnected')
  await page.getByRole('button', { name: 'Pause recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Resume recording', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'pause_recording').at(-1).args)).toEqual({ projectId: 'new-session' })
})

async function beginMockRecording(page: Page, id: string) {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(id => { (window as any).__test.recordingProjectId = id }, id)
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0'); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
}

test('a failed initialization status preserves usable app information and can be retried', async ({ page }) => {
  await mockedDesktop(page, { get_recording_state: 'Recording status is temporarily unavailable' }); await page.goto('/')
  await expect(page.getByRole('alert')).toContainText('Recording status is temporarily unavailable'); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeEnabled(); await expect(page.getByRole('button', { name: 'UI verification recording' })).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.commandErrors = {} }); await page.getByRole('button', { name: 'Retry initialization', exact: true }).click(); await expect(page.getByRole('alert')).toHaveCount(0)
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await expect(page.getByRole('button', { name: 'Start recording', exact: true })).toBeEnabled()
})

test('failed app information has an explicit initialization retry without restarting the app', async ({ page }) => {
  await mockedDesktop(page, { get_app_info: 'The app information request failed' }); await page.goto('/')
  await expect(page.getByRole('alert')).toContainText('The app information request failed'); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeDisabled()
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByRole('alert')).toContainText('The app information request failed')
  await page.evaluate(() => { const state = (window as any).__test; state.commandErrors = {}; state.holdAppInfo = true }); await page.getByRole('button', { name: 'Retry initialization', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishAppInfo)).toBe('function'); await expect(page.getByRole('button', { name: 'Export MP4', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.finishAppInfo()); await expect(page.getByRole('alert')).toHaveCount(0); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeEnabled(); await expect(page.getByRole('button', { name: 'Export MP4', exact: true })).toBeEnabled()
})

test('failed automatic stop preserves its diagnostic without opening an unrelated older project', async ({ page }) => {
  await beginMockRecording(page, 'failed-session')
  await page.evaluate(() => { const state = (window as any).__test; Object.assign(state.recordingState, { active: false, phase: 'idle', projectId: null }); state.holdRecordingState = true; state.finishRecordingState = null })
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingState)).toBe('function')
  const polls = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'get_recording_state').length)
  await page.evaluate(() => { const state = (window as any).__test; state.emit('recording-error', { projectId: 'failed-session', error: 'Capture failed and no usable video could be recovered.' }); state.finishRecordingState() })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'get_recording_state').length)).toBeGreaterThan(polls)
  await expect(page.getByRole('alert')).toContainText('Capture failed and no usable video could be recovered.'); await expect(page.getByLabel('Project name')).toHaveCount(0)
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'open_project'))).toBe(false)
})

test('a missing terminal event opens only the completed recording identity and retains its fault', async ({ page }) => {
  await beginMockRecording(page, 'completed-session')
  await page.evaluate(() => { const state = (window as any).__test; state.addProject('completed-session', 'Completed session result'); state.emit('recording-error', { projectId: 'completed-session', error: 'The camera disconnected; the screen was saved.' }); Object.assign(state.recordingState, { active: false, phase: 'idle', projectId: null }) })
  await expect(page.getByLabel('Project name')).toHaveValue('Completed session result'); await expect(page.getByRole('alert')).toContainText('The camera disconnected; the screen was saved.')
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'open_project').map((call: any) => call.args.id))).toEqual(['completed-session'])
})

test('terminal results before a start acknowledgement are matched to the new session and retain its error', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { const state = (window as any).__test; state.holdRecordingStart = true; state.recordingProjectId = 'rapid-session'; state.endedBeforeStartAck = true })
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0'); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingStart)).toBe('function')
  await page.evaluate(() => { const state = (window as any).__test; const result = { ...state.currentProject(), id: 'rapid-session', name: 'Recovered rapid session', status: 'interrupted' }; state.addProject(result.id, result.name); state.emit('recording-stopped', result); state.emit('recording-error', { projectId: result.id, error: 'The new camera disconnected immediately after startup.' }); state.emit('recording-stopped', state.currentProject()); state.emit('recording-error', { projectId: 'ui-test', error: 'An unrelated old session failed.' }); state.finishRecordingStart() })
  await expect(page.getByLabel('Project name')).toHaveValue('Recovered rapid session'); await expect(page.getByRole('alert')).toContainText('The new camera disconnected immediately after startup.'); await expect(page.getByRole('alert')).not.toContainText('unrelated'); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toHaveCount(0)
  await expect(page.getByText('Recovered after an interrupted recording. Check playback before exporting.', { exact: true })).toBeVisible()
})

test('a delayed health rejection cannot overwrite a newer recording action', async ({ page }) => {
  await beginMockRecording(page, 'health-session')
  await page.evaluate(() => { const state = (window as any).__test; state.recordingStateError = 'Old health snapshot failed'; state.holdRecordingState = true; state.finishRecordingState = null }); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingState)).toBe('function')
  await page.getByRole('button', { name: 'Pause recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Resume recording', exact: true })).toBeEnabled()
  const polls = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'get_recording_state').length)
  await page.evaluate(() => { const state = (window as any).__test; state.recordingStateError = null; state.finishRecordingState() })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'get_recording_state').length)).toBeGreaterThan(polls); await expect(page.getByRole('alert')).toHaveCount(0); await expect(page.getByRole('button', { name: 'Resume recording', exact: true })).toBeEnabled()
})

test('slow valid webcam finalization stays locked past the old deadline and opens a ready project once', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#webcam').selectOption('cam-test'); await page.getByLabel('Countdown', { exact: true }).selectOption('0')
  await page.evaluate(() => { const state = (window as any).__test; state.holdRecordingStop = true; state.emitRecordingStop = true; state.recordingProjectId = 'slow-ready'; state.recordedProject = { ...state.currentProject(), id: 'slow-ready', name: 'Completed webcam recording', status: 'ready' } })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.clock.install(); await page.clock.pauseAt(new Date())
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingStop)).toBe('function')
  await page.clock.runFor(25_000)
  await expect(page.getByRole('status', { name: '' }).filter({ hasText: /^Finalizing recording$/ })).toBeVisible(); await expect(page.getByRole('button', { name: 'Finalizing recording', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeDisabled()
  await page.getByRole('button', { name: 'Finalizing recording', exact: true }).evaluate(button => (button as HTMLButtonElement).click())
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null)); await expect(page.getByRole('alert')).toHaveText('Wait for recording finalization to finish before closing.Dismiss')
  const calls = await page.evaluate(() => (window as any).__test.calls); expect(calls.filter((call: any) => call.name === 'stop_recording')).toHaveLength(1); expect(calls.filter((call: any) => call.name === 'start_recording')).toHaveLength(1); expect(calls.some((call: any) => call.name === 'close_app')).toBe(false)
  await page.getByRole('button', { name: 'Dismiss error', exact: true }).click(); await page.evaluate(() => (window as any).__test.finishRecordingStop()); await expect(page.getByLabel('Project name')).toHaveValue('Completed webcam recording'); await expect(page.getByText('Recovered after an interrupted recording. Check playback before exporting.', { exact: true })).toHaveCount(0); await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeEnabled(); expect(await page.evaluate(() => (window as any).__test.currentProject().status)).toBe('ready')
})

test('actual stop failure remains visible and recovered media retains its interrupted status', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByLabel('Countdown', { exact: true }).selectOption('0')
  await page.evaluate(() => { const state = (window as any).__test; state.recordingStopError = 'Webcam media could not be finalized for test'; state.holdRecordingStop = true })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await page.getByRole('button', { name: 'Stop recording', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRecordingStop)).toBe('function')
  await page.evaluate(() => (window as any).__test.finishRecordingStop()); await expect(page.getByRole('alert')).toHaveCount(1); await expect(page.getByRole('alert')).toContainText('Webcam media could not be finalized for test'); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeEnabled(); await expect(page.getByLabel('Project name')).toHaveCount(0)
  await page.evaluate(() => { const state = (window as any).__test; state.recordingStopError = null; state.holdRecordingStop = false; state.emitRecordingStop = true; state.recordedProject = { ...state.currentProject(), id: 'partial', name: 'Recovered partial recording', status: 'interrupted' }; state.recordingProjectId = 'partial' })
  await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await expect(page.getByLabel('Project name')).toHaveValue('Recovered partial recording'); await expect(page.getByText('Recovered after an interrupted recording. Check playback before exporting.', { exact: true })).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.currentProject().status)).toBe('interrupted')
})

test('finalization at disk capacity keeps the partial recording available for later recovery', async ({ page }) => {
  await beginMockRecording(page, 'capacity-session')
  const diagnostic = 'Recording stopped with an error: There is not enough space on the disk (os error 112). Recovery could not finish. Available media is preserved.'
  await page.evaluate(message => {
    const state = (window as any).__test
    state.recordingState.elapsed = 10802
    state.recordingStopError = message
    state.holdRecordingStop = true
  }, diagnostic)
  await expect(page.getByLabel('Recorded duration')).toHaveText('3:00:02')
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Finalizing recording', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect(page.getByRole('alert')).toContainText('Wait for recording finalization')
  await page.evaluate(() => (window as any).__test.finishRecordingStop())
  await expect(page.getByRole('alert')).toContainText(diagnostic)
  await expect(page.getByLabel('Project name')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeEnabled()
  await page.evaluate(() => {
    const state = (window as any).__test
    state.patchProject({ id: 'capacity-session', name: 'Recovered partial recording', status: 'interrupted' })
    state.recoveryIssues = ['Recovered the available screen video after the disk filled.']
    state.emit('library-updated', null)
    state.emit('recovery-issues-updated', null)
  })
  await expect(page.locator('.recovery-issues summary')).toHaveText('1 project recovery issue')
  await page.getByRole('button', { name: /^Recovered partial recording/ }).click()
  await expect(page.getByLabel('Project name')).toHaveValue('Recovered partial recording')
  await expect(page.getByText('Recovered after an interrupted recording. Check playback before exporting.')).toBeVisible()
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => element.currentTime)).toBeGreaterThan(.15)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name))
  expect(calls.filter(name => name === 'stop_recording')).toHaveLength(1)
  expect(calls).not.toContain('close_app')
  expect(calls).not.toContain('delete_project')
})

test('main window respects externally started finalization without allowing another Stop or close', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { Object.assign((window as any).__test.recordingState, { active: true, projectId: 'external-stop', phase: 'finalizing', elapsed: 62, cameraAvailable: true }) })
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'Pause recording', exact: true })).toBeDisabled(); await expect(page.getByRole('button', { name: 'New recording', exact: true }).first()).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null)); await expect(page.getByRole('alert')).toContainText('Wait for recording finalization to finish before closing.')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => ['close_app', 'stop_recording', 'start_recording'].includes(call.name)))).toBe(false)
})

test('joined webcam gaps stay hidden while its padded media clock continues through the gap', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: '/test-media/camera-short.mp4', cameraOffsetSeconds: 0, cameraIntervals: [{ start: 0, end: .2 }, { start: .5, end: .8 }], edits: { padding: 180, camera: { enabled: true, x: 0, y: .2, width: .1, shape: 'rectangle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const seek = async (time: number) => page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, time)
  const pixel = () => page.locator('canvas').evaluate(element => [...element.getContext('2d')!.getImageData(Math.round(element.width * .05), Math.round(element.height * .2 + element.width * .05), 1, 1).data].slice(0, 3))
  await seek(.1); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
  await seek(.2); await expect.poll(pixel).toEqual([40, 53, 68])
  await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => element.currentTime)).toBeCloseTo(.2, 3)
  await page.evaluate(() => {
    const w = window as any; w.__cameraDraws = []; const original = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function(source: CanvasImageSource, ...args: any[]) { if (source instanceof HTMLVideoElement && source.dataset.track === 'webcam recording') w.__cameraDraws.push(document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!.currentTime); return (original as any).call(this, source, ...args) }
  })
  await page.getByRole('button', { name: 'Play preview', exact: true }).click(); await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => element.paused)).toBe(false)
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => element.currentTime)).toBeGreaterThan(.9)
  const draws = await page.evaluate(() => (window as any).__cameraDraws as number[])
  expect(draws.some(time => time >= .5 && time < .8)).toBe(true); expect(draws.some(time => time >= .2 && time < .5 || time >= .8)).toBe(false)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click(); await seek(.8); await expect.poll(pixel).toEqual([40, 53, 68])
})

test('manual source gains support keyboard max, independent volume, undo, reset and saved reopen', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav', edits: { microphoneVolume: .5, systemVolume: .75 } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const mic = page.getByRole('slider', { name: 'Microphone source gain', exact: true }); const system = page.getByRole('slider', { name: 'Computer audio source gain', exact: true })
  await expect(mic).toHaveValue('1'); await expect(system).toHaveValue('1'); await expect(mic).toHaveAttribute('min', '.01'); await expect(mic).toHaveAttribute('max', '2'); await expect(mic).toHaveAttribute('step', '.01')
  await mic.focus(); await page.keyboard.press('End'); await expect(mic).toHaveValue('2'); await expect(mic).toHaveAttribute('aria-valuetext', '2.00 times'); await expect(system).toHaveValue('1')
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.slice(2).map((node: GainNode) => node.gain.value))).toEqual([1, .75])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(mic).toHaveValue('1'); await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(mic).toHaveValue('2')
  await system.focus(); await page.keyboard.press('End'); await expect(system).toHaveValue('2'); await page.keyboard.press('ArrowLeft'); await expect(system).toHaveValue('1.99'); await page.keyboard.press('End'); await expect(system).toHaveValue('2')
  await page.locator('#mic-volume').fill('0'); await page.locator('#mic-volume').blur(); await expect.poll(() => page.evaluate(() => (window as any).__audioGains.slice(2).map((node: GainNode) => node.gain.value))).toEqual([0, 1.5])
  await page.getByRole('button', { name: 'Reset computer audio gain', exact: true }).click(); await expect(system).toHaveValue('1'); await expect(mic).toHaveValue('2'); await expect(page.locator('#system-volume')).toHaveValue('0.75')
  await page.getByRole('button', { name: 'Save', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved.edits.audioProcessing)).toMatchObject({ microphoneGain: 2, systemGain: 1 })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await expect(mic).toHaveValue('2'); await expect(system).toHaveValue('1'); await expect(page.locator('#mic-volume')).toHaveValue('0')
  await mic.focus(); await page.keyboard.press('Home'); await expect(mic).toHaveValue('0.01'); await page.getByRole('button', { name: 'Reset microphone gain', exact: true }).click(); await expect(mic).toHaveValue('1')
})

test('legacy gains above two remain real and unchanged until explicit edit, reset or normalize', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav', edits: { microphoneVolume: .25, systemVolume: .5, audioProcessing: { microphoneGain: 4, systemGain: 8, ducking: { enabled: false, amountDb: 12 } } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const micRegion = page.getByRole('region', { name: 'Microphone settings' }); const systemRegion = page.getByRole('region', { name: 'Computer audio settings' })
  await expect(micRegion).toContainText('4.00x'); await expect(systemRegion).toContainText('8.00x'); await expect(page.getByRole('slider', { name: 'Microphone source gain', exact: true })).toHaveCount(0); await expect(page.getByRole('slider', { name: 'Computer audio source gain', exact: true })).toHaveCount(0)
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.slice(2).map((node: GainNode) => node.gain.value))).toEqual([1, 4]); expect(await page.evaluate(() => (window as any).__test.calls.filter((c: any) => c.name === 'save_project_edits').length)).toBe(0)
  await page.getByRole('button', { name: 'Set microphone gain to 2.00x', exact: true }).click(); const mic = page.getByRole('slider', { name: 'Microphone source gain', exact: true }); await expect(mic).toHaveValue('2'); await expect(mic).toBeFocused(); await expect(systemRegion).toContainText('8.00x')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(micRegion).toContainText('4.00x'); await expect(mic).toHaveCount(0); await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(mic).toHaveValue('2')
  await page.getByRole('button', { name: 'Reset computer audio gain', exact: true }).click(); await expect(page.getByRole('slider', { name: 'Computer audio source gain', exact: true })).toHaveValue('1'); await expect(page.locator('#system-volume')).toHaveValue('0.5')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(systemRegion).toContainText('8.00x')
  await page.evaluate(() => { (window as any).__test.analysisResult = { sourceDuration: 12, bins: [{ min: -.1, max: .1 }], peak: .1, rms: .01, analyzedSeconds: 12, recommendedGain: 8 } })
  await page.getByRole('button', { name: 'Analyze computer audio level', exact: true }).click(); await page.getByRole('button', { name: 'Normalize computer audio level', exact: true }).click(); await expect(page.getByRole('slider', { name: 'Computer audio source gain', exact: true })).toHaveValue('2')
})

test('a legacy gain just above the manual maximum displays its full saved value', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', edits: { audioProcessing: { microphoneGain: 2.001, systemGain: 1, ducking: { enabled: false, amountDb: 12 } } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); const region = page.getByRole('region', { name: 'Microphone settings' })
  await expect(region).toContainText('2.001x'); await expect(page.getByRole('slider', { name: 'Microphone source gain', exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: 'Set microphone gain to 2.00x', exact: true }).click(); await expect(page.getByRole('slider', { name: 'Microphone source gain', exact: true })).toHaveValue('2'); await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(region).toContainText('2.001x')
})

test('Normalize joins an automatic scan already in progress instead of cancelling or repeating it', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.evaluate(() => { const t = (window as any).__test; t.holdAnalysis = true; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishAnalysis)).toBe('function'); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await page.getByRole('button', { name: 'Normalize microphone level', exact: true }).click()
  await page.evaluate(() => (window as any).__test.finishAnalysis()); await expect(page.getByRole('slider', { name: 'Microphone source gain', exact: true })).toHaveValue('1.26')
  const calls = await page.evaluate(() => (window as any).__test.calls); expect(calls.filter((c: any) => c.name === 'analyze_audio' && c.args.source === 'microphone')).toHaveLength(1); expect(calls.filter((c: any) => c.name === 'cancel_audio_analysis')).toHaveLength(0)
})

test('manual audio gain controls remain visible and keyboard focused in short dark and light inspectors', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 640 }); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav' })); await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme)
    const gain = page.getByRole('slider', { name: 'Microphone source gain', exact: true }); await gain.focus(); await page.keyboard.press('ArrowRight'); await expect(gain).toBeFocused(); expect(await gain.evaluate(element => getComputedStyle(element).outlineStyle)).toBe('solid')
    if (theme === 'light') expect(await gain.evaluate(element => getComputedStyle(element).accentColor)).toBe('rgb(138, 88, 0)')
    const system = page.getByRole('slider', { name: 'Computer audio source gain', exact: true }); await system.focus(); await page.keyboard.press('End'); await expect(system).toHaveValue('2'); const box = await system.boundingBox(); expect(box!.x + box!.width).toBeLessThanOrEqual(900); expect(box!.y + box!.height).toBeLessThanOrEqual(640)
    await page.screenshot({ path: `artifacts/ui/source-gain-${theme}-900.png` })
  }
})

test('computer and music ducking save independently with one shared keyboard reduction control', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav', edits: { music: [{ id: 'music', name: 'Music bed', path: '/test-media/audio.wav', start: 0, sourceStart: 0, sourceDuration: 12, duration: 12, volume: .2, fadeIn: 0, fadeOut: 0 }], audioProcessing: { microphoneGain: 1, systemGain: 1, ducking: { enabled: true, amountDb: 12 } } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const computer = page.getByLabel('Lower computer audio while microphone speaks'); const music = page.getByLabel('Lower music while microphone speaks'); const amount = page.getByRole('slider', { name: 'Reduction', exact: true })
  const routing = () => page.evaluate(() => (window as any).__audioMessages.filter((item: any) => item.name === 'screenrecord-music-mixer' && typeof item.message === 'object' && 'enabled' in item.message).at(-1)?.message)
  await expect(computer).not.toBeChecked(); await expect(music).toBeChecked(); await expect.poll(routing).toEqual({ enabled: true, systemEnabled: false, amountDb: 12 })
  await computer.focus(); await page.keyboard.press('Space'); await expect(computer).toBeChecked(); await expect(music).toBeChecked(); await expect.poll(routing).toEqual({ enabled: true, systemEnabled: true, amountDb: 12 })
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(computer).not.toBeChecked(); await expect(music).toBeChecked(); await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(computer).toBeChecked()
  await music.uncheck(); await amount.focus(); await page.keyboard.press('End'); await expect(amount).toHaveValue('24'); await page.keyboard.press('ArrowLeft'); await expect(amount).toHaveValue('23'); await expect.poll(routing).toEqual({ enabled: false, systemEnabled: true, amountDb: 23 })
  await page.getByRole('button', { name: 'Save', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved.edits.audioProcessing.ducking)).toEqual({ enabled: false, systemEnabled: true, amountDb: 23 })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await expect(computer).toBeChecked(); await expect(music).not.toBeChecked(); await expect(amount).toHaveValue('23')
  await computer.uncheck(); await expect(amount).toBeDisabled(); await music.check(); await expect(amount).toBeEnabled(); await expect.poll(routing).toEqual({ enabled: true, systemEnabled: false, amountDb: 23 })
})

test('computer ducking works without imported music and fits short dark and light inspectors', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 640 }); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav' }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await expect(page.getByLabel('Lower music while microphone speaks')).toBeDisabled(); const computer = page.getByLabel('Lower computer audio while microphone speaks'); await expect(computer).toBeEnabled(); await computer.check()
  const amount = page.getByRole('slider', { name: 'Reduction', exact: true }); await expect(amount).toBeEnabled()
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme }, theme)
    await amount.focus(); await page.keyboard.press('ArrowRight'); await expect(amount).toBeFocused(); expect(await amount.evaluate(element => getComputedStyle(element).outlineStyle)).toBe('solid')
    if (theme === 'light') expect(await amount.evaluate(element => getComputedStyle(element).accentColor)).toBe('rgb(138, 88, 0)')
    const box = await amount.boundingBox(); expect(box!.x + box!.width).toBeLessThanOrEqual(900); expect(box!.y + box!.height).toBeLessThanOrEqual(640)
    await page.screenshot({ path: `artifacts/ui/system-ducking-${theme}-900.png` })
  }
})

test('ducking controls require their own target and recorded microphone without changing saved flags', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ systemAudioPath: '/test-media/audio.wav', microphonePath: null, edits: { audioProcessing: { microphoneGain: 1, systemGain: 1, ducking: { enabled: true, systemEnabled: true, amountDb: 18 } } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await expect(page.getByLabel('Lower computer audio while microphone speaks')).toBeDisabled(); await expect(page.getByLabel('Lower music while microphone speaks')).toBeDisabled(); await expect(page.getByRole('slider', { name: 'Reduction', exact: true })).toBeDisabled()
  await expect.poll(() => page.evaluate(() => (window as any).__audioMessages.filter((item: any) => item.name === 'screenrecord-music-mixer' && typeof item.message === 'object' && 'enabled' in item.message).at(-1)?.message)).toEqual({ enabled: false, systemEnabled: false, amountDb: 18 })
  expect(await page.evaluate(() => (window as any).__test.currentProject().edits.audioProcessing.ducking)).toEqual({ enabled: true, systemEnabled: true, amountDb: 18 })
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: null, edits: { music: [{ id: 'music', name: 'Music bed', path: '/test-media/audio.wav', start: 0, sourceStart: 0, sourceDuration: 12, duration: 12, volume: .2, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await expect(page.getByLabel('Lower computer audio while microphone speaks')).toBeDisabled(); await expect(page.getByLabel('Lower music while microphone speaks')).toBeEnabled(); await expect(page.getByRole('slider', { name: 'Reduction', exact: true })).toBeEnabled()
})

test('vertical guides follow the output canvas and pan without altering pixels, edits or camera handles', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  const open = page.getByRole('button', { name: 'UI verification recording' })
  await open.click()
  const choice = page.getByLabel('Safe area', { exact: true })
  await expect(choice).toHaveCount(0)
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { aspectRatio: '9:16' } }))
  await open.click()
  const canvas = page.getByLabel('Edited video preview', { exact: true })
  await expect(page.getByText('Loading video...', { exact: true })).toHaveCount(0)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toHaveCount(0)
  async function canvasPixels() {
    return canvas.evaluate(element => {
      const source = element as HTMLCanvasElement
      const pixels = source.getContext('2d')!.getImageData(0, 0, source.width, source.height).data
      let sum = 0
      for (const channel of pixels) sum = Math.imul(sum, 31) + channel | 0
      return sum
    })
  }
  const original = await canvasPixels()
  await choice.selectOption('tiktok')
  const guide = page.locator('.safe-area-guide')
  await expect(guide).toHaveAttribute('data-guide', 'tiktok')
  async function measure() {
    return guide.evaluate(element => {
      const frame = element.parentElement!.querySelector('canvas')!.getBoundingClientRect()
      const area = element.querySelector('.safe-interior')!.getBoundingClientRect()
      return [area.left - frame.left, frame.right - area.right, area.top - frame.top, frame.bottom - area.bottom].map((value, i) => Math.round(value / (i < 2 ? frame.width : frame.height) * 100))
    })
  }
  expect(await measure()).toEqual([8, 20, 10, 25])
  expect(await canvasPixels()).toBe(original)
  await page.getByRole('tab', { name: 'Camera', exact: true }).click()
  const handle = page.getByRole('button', { name: 'Move camera', exact: true })
  await expect(handle).toBeVisible()
  expect(await handle.evaluate(element => {
    const b = element.getBoundingClientRect()
    return element.contains(document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2))
  })).toBe(true)
  await choice.selectOption('reels')
  await page.getByRole('button', { name: /Switch to light/ }).click()
  await page.getByLabel('Preview scale', { exact: true }).selectOption('200')
  await page.getByRole('region', { name: 'Video preview viewport' }).evaluate(element => element.scrollTo(300, 500))
  await expect.poll(measure).toEqual([6, 16, 14, 35])
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await choice.selectOption('off'); await expect(guide).toHaveCount(0)
  await page.evaluate(() => (window as any).__test.patchProject({ width: 1080, height: 1920, edits: { aspectRatio: 'source' } }))
  await open.click(); await expect(choice).toBeVisible()
})

test('volume points preserve source time through cuts, support keyboard and cancelled drag, and reopen', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav',
    edits: { videoClips: [{ id: 'first', start: 0, end: 3 }, { id: 'second', start: 5, end: 12 }] },
  }))
  const open = page.getByRole('button', { name: 'UI verification recording' })
  await open.click()
  const tools = page.getByRole('button', { name: 'Volume keyframes', exact: true })
  await tools.click()
  const track = page.getByLabel('Volume keyframe track', { exact: true })
  await track.selectOption('system')
  const add = page.getByRole('button', { name: 'Add at playhead' })
  await add.click()
  const time = page.getByRole('spinbutton', { name: 'Keyframe time (seconds)' })
  const level = page.getByRole('spinbutton', { name: 'Keyframe level (%)' })
  await time.fill('4'); await time.press('Enter'); await time.blur()
  await level.fill('25'); await level.press('Enter'); await level.blur()
  await add.click(); await level.fill('100'); await level.press('Enter'); await level.blur()
  const latePoint = page.getByRole('button', { name: 'Computer volume point at 4 seconds, 25%', exact: true })
  await latePoint.focus(); await latePoint.press('ArrowUp')
  await expect(level).toHaveValue('26')
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(latePoint).toBeVisible()
  await latePoint.scrollIntoViewIfNeeded()
  const bounds = (await latePoint.boundingBox())!
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
  await page.mouse.down(); await page.mouse.move(bounds.x + 56, bounds.y - 7, { steps: 4 })
  await expect(latePoint).toHaveCount(0)
  await page.keyboard.press('Escape'); await page.mouse.up()
  await expect(latePoint).toBeVisible()
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
  await page.mouse.down(); await page.mouse.move(bounds.x + 56, bounds.y - 7, { steps: 4 })
  await expect(latePoint).toHaveCount(0)
  await page.keyboard.press('Tab')
  await expect(latePoint).toBeVisible()
  await level.fill('33'); await level.press('Enter'); await level.blur()
  await page.mouse.up(); await page.keyboard.press('Escape')
  await expect(page.getByRole('button', { name: 'Computer volume point at 4 seconds, 33%', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(latePoint).toBeVisible()
  const savedPoints = () => page.evaluate(() => (window as any).__test.saved?.edits.audioProcessing.volumeKeyframes?.system?.map((key: any) => [key.time, key.volume]))
  await expect.poll(savedPoints).toEqual([[0, 1], [6, .25]])
  await open.click(); await tools.click(); await track.selectOption('system')
  await expect(latePoint).toBeVisible()
  await page.getByRole('button', { name: 'Reset curve' }).click()
  await expect(page.locator('.volume-point')).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(page.locator('.volume-point')).toHaveCount(2)
})

test('audition selects a range in either direction and leaves project edits untouched', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const audition = page.getByRole('button', { name: 'Audition', exact: true })
  await audition.click()
  await expect(page.getByLabel('Preview audio', { exact: true })).toHaveValue('mix')
  await expect(page.getByRole('option', { name: 'Microphone only' })).toBeDisabled()
  const lane = page.locator('.audition-lane')
  await lane.scrollIntoViewIfNeeded()
  const bounds = (await lane.boundingBox())!
  const y = bounds.y + bounds.height / 2
  await page.mouse.move(bounds.x + bounds.width * .75, y)
  await page.mouse.down()
  await page.mouse.move(bounds.x + bounds.width * .25, y, { steps: 4 })
  await page.mouse.up()
  const start = page.getByRole('spinbutton', { name: 'Loop start (seconds)' })
  const end = page.getByRole('spinbutton', { name: 'Loop end (seconds)' })
  expect(Number(await start.inputValue())).toBeCloseTo(3, 1)
  expect(Number(await end.inputValue())).toBeCloseTo(9, 1)
  await start.fill('2.375'); await start.press('Tab')
  await end.fill('5'); await end.press('Tab')
  await page.getByLabel('Loop selection', { exact: true }).check()
  await expect(start).toHaveValue('2.375')
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await audition.click(); await audition.click()
  await expect(page.getByLabel('Loop selection', { exact: true })).not.toBeChecked()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'save_project_edits').length)).toBe(0)
})

test('recorded waveforms appear automatically and normalization preserves volume with one undo', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/audio.wav', systemAudioPath: '/test-media/audio.wav', edits: { microphoneVolume: .5, music: [{ id: 'music', name: 'Music bed', path: '/test-media/audio.wav', start: 0, sourceStart: 0, sourceDuration: 12, duration: 12, volume: .2, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByRole('img', { name: 'Microphone recorded waveform' })).toBeVisible(); await expect(page.getByRole('img', { name: 'Computer recorded waveform' })).toBeVisible(); await expect(page.getByRole('img', { name: 'Music bed music waveform' })).toBeVisible()
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const microphoneAnalyses = () => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'analyze_audio' && call.args.source === 'microphone').length)
  const initialAnalyses = await microphoneAnalyses()
  await page.getByRole('button', { name: 'Normalize microphone level', exact: true }).click()
  expect(await microphoneAnalyses()).toBe(initialAnalyses)
  await expect(page.getByRole('region', { name: 'Microphone settings' })).toContainText('1.26x'); await expect(page.locator('#mic-volume')).toHaveValue('0.5')
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains[2].gain.value)).toBeCloseTo(.6294627, 6)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.getByRole('region', { name: 'Microphone settings' })).toContainText('1.00x')
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await page.getByRole('button', { name: 'Reset microphone gain', exact: true }).click(); await expect(page.getByRole('region', { name: 'Microphone settings' })).toContainText('1.00x')
  await page.getByLabel('Lower music while microphone speaks').check(); await page.locator('#ducking-amount').fill('18')
  await page.getByRole('button', { name: 'Save', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved.edits.audioProcessing.ducking)).toEqual({ enabled: true, amountDb: 18 })
  await page.screenshot({ path: 'artifacts/ui/audio-tools-dark.png' })
  await page.getByRole('button', { name: /Switch to light/ }).click(); await page.screenshot({ path: 'artifacts/ui/audio-tools-light.png' })
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click(); await page.getByLabel('End (seconds)', { exact: true }).fill('8'); await page.getByLabel('End (seconds)', { exact: true }).blur()
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await expect.poll(microphoneAnalyses).toBe(initialAnalyses + 1)
  await expect.poll(() => page.evaluate(() => (window as any).__test.analysisSnapshot.trimEnd)).toBe(8)
  const refreshFlags = () => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'analyze_audio' && call.args.source === 'microphone').map((call: any) => call.args.forceRefresh))
  expect((await refreshFlags()).every((value: boolean) => value === false)).toBe(true)
  await page.getByRole('button', { name: 'Analyze microphone level', exact: true }).click(); await expect(page.getByRole('region', { name: 'Microphone settings' })).toContainText('Raw RMS')
  expect((await refreshFlags()).slice(-2)).toEqual([false, true])
})

test('switching projects drains a late analysis and discards queued statistics for the old cuts', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => {
    const w = window as any; const state = w.__test; const original = w.__TAURI_INTERNALS__.invoke
    state.patchProject({ microphonePath: '/test-media/audio.wav' }); state.addProject('analysis-next', 'Next audio project'); state.emit('library-updated', null)
    w.__TAURI_INTERNALS__.invoke = async (name: string, args: any) => {
      if (name === 'cancel_audio_analysis') { state.calls.push({ name, args }); return }
      if (name === 'analyze_audio') {
        state.calls.push({ name, args })
        if (args.id === 'ui-test') await new Promise<void>(resolve => { state.finishOldReader = resolve })
        return { sourceDuration: 12, bins: [{ min: -.25, max: .25 }], peak: args.id === 'ui-test' ? .9 : .25, rms: .1, analyzedSeconds: 12, recommendedGain: args.id === 'ui-test' ? 1.1 : 1.5 }
      }
      return original(name, args)
    }
  })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishOldReader)).toBe('function')
  await page.getByLabel('End (seconds)', { exact: true }).fill('8'); await page.getByLabel('End (seconds)', { exact: true }).blur(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.trimEnd)).toBe(8)
  await page.getByRole('button', { name: /^Next audio project/ }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'cancel_audio_analysis'))).toBe(true)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'open_project').map((call: any) => call.args.id))).toEqual(['ui-test'])
  await page.evaluate(() => (window as any).__test.finishOldReader()); await expect(page.getByLabel('Project name')).toHaveValue('Next audio project'); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await expect(page.getByRole('region', { name: 'Microphone settings' })).toContainText('-12.0 dBFS')
  const scans = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'analyze_audio'))
  expect(scans.map((scan: any) => scan.args.id)).toEqual(['ui-test', 'analysis-next'])
  await page.evaluate(old => (window as any).__test.emit('audio-analysis-progress', { jobId: old.jobId, projectId: old.id, source: old.source, progress: .8 }), scans[0].args)
  await page.getByRole('button', { name: 'Normalize microphone level', exact: true }).click(); await expect(page.getByRole('slider', { name: 'Microphone source gain', exact: true })).toHaveValue('1.5')
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'analyze_audio').length)).toBe(2)
})

test('analysis cancellation precedes native close and stale analysis cannot apply normalization', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.holdAnalysis = true; state.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishAnalysis)).toBe('function')
  await page.evaluate(() => (window as any).__test.emit('close-blocked', 'Cancel audio analysis before closing.'))
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(true)
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name)); expect(calls.indexOf('cancel_audio_analysis')).toBeLessThan(calls.indexOf('close_app'))
  await page.evaluate(() => { const state = (window as any).__test; state.holdAnalysis = false; state.analysisResult = { sourceDuration: 12, bins: [{ min: 0, max: 0 }], peak: 0, rms: 0, analyzedSeconds: 12, recommendedGain: 1 } })
  await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await page.getByRole('button', { name: 'Analyze microphone level', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Normalize microphone level', exact: true })).toBeDisabled(); await expect(page.getByText('No signal in the kept video.', { exact: true })).toBeVisible()
})

test('failed and cancelled normalization preserve gain and ducking requires a microphone', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.analysisError = 'Audio fixture cannot be decoded'; state.patchProject({ systemAudioPath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await expect(page.getByText('Audio fixture cannot be decoded', { exact: true })).toBeVisible(); await expect(page.getByLabel('Lower music while microphone speaks')).toBeDisabled()
  await page.getByRole('button', { name: 'Normalize computer audio level', exact: true }).click(); await expect(page.getByRole('region', { name: 'Computer audio settings' })).toContainText('1.00x')
  await page.evaluate(() => { const state = (window as any).__test; state.analysisError = null; state.holdAnalysis = true; state.finishAnalysis = null })
  await page.getByRole('button', { name: 'Normalize computer audio level', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishAnalysis)).toBe('function')
  await page.locator('.notice-banner').getByRole('button', { name: 'Cancel analysis', exact: true }).click(); await expect(page.getByRole('button', { name: 'Normalize computer audio level', exact: true })).toBeEnabled()
  await expect(page.getByRole('region', { name: 'Computer audio settings' })).toContainText('1.00x')
  await page.evaluate(() => { (window as any).__test.holdAnalysis = false }); await page.getByRole('button', { name: 'Analyze computer audio level', exact: true }).click(); await expect(page.getByRole('img', { name: 'Computer recorded waveform' })).toBeVisible()
})

test('microphone preflight is opt-in, shows levels and releases on device change and timeout', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  await page.locator('#microphone').selectOption('mic-test'); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_microphone_meter'))).toBe(false)
  await page.getByRole('button', { name: 'Test microphone', exact: true }).click(); await expect(page.getByRole('meter', { name: 'Microphone peak level' })).toHaveAttribute('aria-valuetext', '-6.0 dBFS')
  await page.evaluate(() => { (window as any).__test.meterPeak = 1 }); await expect(page.getByText('Clipping risk', { exact: true })).toBeVisible()
  expect(await page.locator('audio').count()).toBe(0)
  await page.locator('#microphone').selectOption({ label: 'Off' }); await expect(page.getByRole('button', { name: 'Test microphone', exact: true })).toBeDisabled()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'stop_microphone_meter'))).toBe(true)
  await page.locator('#microphone').selectOption('mic-test'); await page.getByRole('button', { name: 'Test microphone', exact: true }).click()
  await page.evaluate(() => { (window as any).__test.meterEnded = true }); await expect(page.getByText('30-second test ended', { exact: true })).toBeVisible()
})

test('recording setup waits for microphone test cleanup before Cancel, Escape and Start', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  for (const action of ['cancel', 'escape', 'start']) {
    await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#microphone').selectOption('mic-test'); await page.getByRole('button', { name: 'Test microphone', exact: true }).click(); await expect(page.getByRole('button', { name: 'Stop test', exact: true })).toBeVisible()
    await page.evaluate(() => { const state = (window as any).__test; state.holdMeterStop = true; state.finishMeterStop = null })
    if (action === 'escape') await page.keyboard.press('Escape'); else await page.getByRole('dialog', { name: 'New recording' }).getByRole('button', { name: action === 'start' ? 'Start recording' : 'Cancel', exact: true }).click()
    await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishMeterStop)).toBe('function'); await expect(page.getByRole('dialog', { name: 'New recording' })).toBeVisible()
    expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
    await page.evaluate(() => { const state = (window as any).__test; state.holdMeterStop = false; state.finishMeterStop() }); await expect(page.getByRole('dialog', { name: 'New recording' })).toBeHidden()
  }
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeVisible()
})

test('a delayed microphone test start cannot reopen after setup closes', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#microphone').selectOption('mic-test')
  await page.evaluate(() => { (window as any).__test.holdMeterStart = true }); await page.getByRole('button', { name: 'Test microphone', exact: true }).click()
  await page.getByRole('dialog', { name: 'New recording' }).getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'stop_microphone_meter'))).toBe(true)
  await page.evaluate(() => { const state = (window as any).__test; state.holdMeterStart = false; state.finishMeterStart() }); await expect(page.getByRole('dialog', { name: 'New recording' })).toBeHidden()
  const calls = await page.evaluate(() => (window as any).__test.calls)
  expect(calls.filter((call: any) => call.name === 'stop_microphone_meter').length).toBeGreaterThanOrEqual(2); expect(calls.some((call: any) => call.name === 'get_microphone_meter')).toBe(false)
})

test('failed microphone cleanup prevents recording and can be retried without opening another device', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#microphone').selectOption('mic-test'); await page.getByRole('button', { name: 'Test microphone', exact: true }).click()
  await page.evaluate(() => { (window as any).__test.meterStopError = 'Microphone cleanup failed for test' }); await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Retry stop test', exact: true })).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.evaluate(() => { (window as any).__test.meterStopError = null }); await page.getByRole('button', { name: 'Retry stop test', exact: true }).click(); await expect(page.getByRole('button', { name: 'Test microphone', exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'start_microphone_meter').length)).toBe(1)
  await page.getByRole('button', { name: 'Start recording', exact: true }).click(); await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeVisible()
})

test('video split and ripple delete use output time, persist source clips and undo once', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '3'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await expect(page.locator('video[data-track="screen recording"]')).toHaveJSProperty('currentTime', 3)
  await page.getByLabel('Edited video preview').focus(); await page.keyboard.press('s')
  await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(2)
  await expect(page.getByLabel('Clip source start (s)')).toHaveValue('3')
  await page.getByLabel('Project name').focus(); await page.keyboard.press('Delete')
  await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(2)
  await page.getByRole('button', { name: /^Select video clip 1,/ }).click(); await page.keyboard.press('Delete')
  await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(1)
  await expect(page.getByLabel('Playhead position in seconds')).toHaveAttribute('max', '9')
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.videoClips.map((clip: any) => [clip.start, clip.end]))).toEqual([[3, 12]])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(2)
  await expect(page.getByLabel('Playhead position in seconds')).toHaveAttribute('max', '12')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(1); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: /^Select video clip 1,/ }).click(); await page.getByRole('button', { name: 'Ripple delete', exact: true }).click()
  await expect(page.getByRole('status').filter({ hasText: 'Keep at least one video clip.' })).toBeVisible()
})

test('range cuts project source-anchored effects and only expose their true trim handles', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { trimEnd: 9, zooms: [{ id: 'zoom-span', start: 2, end: 8, scale: 2, x: .5, y: .5, followCursor: false }], music: [{ id: 'song', name: 'Music across cut', path: '/test-media/audio.wav', start: 2, sourceStart: 0, duration: 6, sourceDuration: 12, volume: .5, fadeIn: 0, fadeOut: 0 }], subtitles: [{ id: 'crossing', start: 2, end: 7, text: 'This sentence crosses the deleted section' }, { id: 'kept', start: 7, end: 8, text: 'Keep this caption' }], redactions: [{ id: 'mask-span', start: 2, end: 8, x: .2, y: .2, width: .1, height: .1, mode: 'solid', strength: 16 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('button', { name: 'Cut range...', exact: true }).click(); await page.getByLabel('Output start (seconds)').fill('3'); await page.getByLabel('Output end (seconds)').fill('6'); await page.getByRole('button', { name: 'Cut range', exact: true }).click()
  await expect(page.getByLabel('Playhead position in seconds')).toHaveAttribute('max', '6')
  await expect(page.getByText('Captions crossing cuts are hidden.', { exact: false })).toBeVisible()
  await expect(page.getByRole('button', { name: /^Edit subtitles 1:/ })).toHaveCount(1)
  await expect(page.getByRole('button', { name: /^Edit masks 1:/ })).toHaveCount(2)
  await expect(page.getByRole('button', { name: 'Trim end of masks 1', exact: true })).toHaveCount(1)
  await expect(page.getByRole('button', { name: 'Trim start of music 1', exact: true })).toHaveCount(1)
  const end = page.getByRole('button', { name: 'Trim end of masks 1', exact: true }); await end.scrollIntoViewIfNeeded(); const bounds = (await end.boundingBox())!; const width = await page.locator('.timeline-plane').evaluate(el => el.getBoundingClientRect().width)
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2); await page.mouse.down(); await page.mouse.move(bounds.x + bounds.width / 2 - width / 6, bounds.y + bounds.height / 2, { steps: 8 }); await page.mouse.up()
  await expect(page.getByLabel('Region end (s)')).toHaveValue('7.000')
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.redactions[0].end)).toBe(8)
  const saved = await page.evaluate(() => (window as any).__test.saved.edits)
  expect(saved.music[0]).toMatchObject({ start: 2, duration: 6, sourceStart: 0 }); expect(saved.subtitles).toHaveLength(2); expect(saved.zooms[0]).toMatchObject({ start: 2, end: 8 })
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '3'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(el => (el as HTMLVideoElement).currentTime)).toBe(6)
  await page.getByRole('button', { name: 'Cut range...', exact: true }).click(); await page.getByLabel('Output start (seconds)').fill('0'); await page.getByLabel('Output end (seconds)').fill('6'); await page.getByRole('button', { name: 'Cut range', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'Cut video range' }).getByRole('alert')).toContainText('Keep at least 0.1 seconds')
})

test('audition solo monitors use the post-fader buses without changing saved edits or stopping the detector track', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/mix-audio.wav', systemAudioPath: '/test-media/mix-audio.wav', edits: { microphoneVolume: .1, systemVolume: .2, music: [{ id: 'music', name: 'Music bed', path: '/test-media/mix-audio.wav', start: 0, sourceStart: 0, sourceDuration: 12, duration: 12, volume: .1, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('button', { name: 'Audition', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.length)).toBe(5)
  await page.evaluate(async source => {
    const w = window as any; const context = w.__audioContexts[0]; const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' })); await context.audioWorklet.addModule(url); URL.revokeObjectURL(url)
    const tap = new AudioWorkletNode(context, 'pcm-tap'); w.__monitorPcm = { count: 0, energy: 0 }; w.__monitorAfter = 0
    tap.port.onmessage = event => { if (context.currentTime > w.__monitorAfter) { w.__monitorPcm.count += event.data.count; w.__monitorPcm.energy += event.data.energy } }; w.__audioGains[1].connect(tap); tap.connect(context.destination)
  }, pcmTapSource)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  for (const [monitor, expected] of [['mix', .24], ['system', .12], ['microphone', .06], ['mix', .24]] as const) {
    await page.getByLabel('Preview audio', { exact: true }).selectOption(monitor)
    await page.evaluate(() => { const w = window as any; w.__monitorAfter = w.__audioContexts[0].currentTime + .08; w.__monitorPcm = { count: 0, energy: 0 } })
    await expect.poll(() => page.evaluate(() => (window as any).__monitorPcm.count)).toBeGreaterThan(24_000)
    const rms = await page.evaluate(() => { const pcm = (window as any).__monitorPcm; return Math.sqrt(pcm.energy / pcm.count) }); expect(rms).toBeCloseTo(expected, 3)
    await expect.poll(() => page.locator('audio[data-track="microphone recording"]').evaluate(element => element.paused)).toBe(false)
  }
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('suspended')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'save_project_edits'))).toBe(false)
  await page.getByLabel('Preview audio', { exact: true }).selectOption('system'); await page.getByRole('button', { name: 'Audition', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioMessages.filter((item: any) => item.message?.monitor).at(-1)?.message.monitor)).toBe('mix')
})

test('audition loops output ranges across cuts with decoded restart and no out-of-range PCM or red frames', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page)
  await page.addInitScript(() => {
    const w = window as any; w.__loopRed = 0; w.__loopDraws = 0; w.__loopStarts = 0; const draw = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (image: CanvasImageSource, ...args: any[]) {
      const result = (draw as any).call(this, image, ...args)
      if (w.__countLoop && this.canvas.classList.contains('preview-canvas')) { w.__loopDraws++; const p = this.getImageData(Math.round(this.canvas.width * .4), Math.round(this.canvas.height * .4), 1, 1).data; if (p[0] > 160 && p[1] < 60 && p[2] < 60) w.__loopRed++ }
      return result
    } as typeof draw
  })
  await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ duration: 6, screenPath: '/test-media/cut-zones.mp4', cameraPath: '/test-media/cut-zones.mp4', systemAudioPath: '/test-media/loop-zones.wav', edits: { trimEnd: 6, padding: 0, cornerRadius: 0, zooms: [], showCursor: false, videoClips: [{ id: 'first', start: 0, end: .9977 }, { id: 'second', start: 2.0023, end: 2.9977 }], music: [{ id: 'tone', name: 'Outside-loop tone', path: '/test-media/loop-zones.wav', sourceDuration: 6, start: 0, sourceStart: 0, duration: 6, volume: 1, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('button', { name: 'Audition', exact: true }).click(); await page.getByLabel('Loop start (seconds)').fill('.2023'); await page.getByLabel('Loop start (seconds)').blur(); await page.getByLabel('Loop end (seconds)').fill('1.7931'); await page.getByLabel('Loop end (seconds)').blur(); await page.getByLabel('Loop selection', { exact: true }).check()
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.length)).toBe(4)
  await page.evaluate(async source => {
    const w = window as any; const context = w.__audioContexts[0]; const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' })); await context.audioWorklet.addModule(url); URL.revokeObjectURL(url)
    const tap = new AudioWorkletNode(context, 'pcm-tap'); w.__loopPcm = { count: 0, peak: 0 }; tap.port.onmessage = event => { w.__loopPcm.count += event.data.count; w.__loopPcm.peak = Math.max(w.__loopPcm.peak, event.data.peak) }; w.__audioGains[1].connect(tap); tap.connect(context.destination)
    const screen = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!; screen.addEventListener('seeked', () => { if (Math.abs(screen.currentTime - .2023) < .001) w.__loopStarts++ }); w.__countLoop = true
  }, pcmTapSource)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__loopStarts), { timeout: 15_000 }).toBeGreaterThanOrEqual(3)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('suspended')
  const evidence = await page.evaluate(() => ({ starts: (window as any).__loopStarts, draws: (window as any).__loopDraws, red: (window as any).__loopRed, pcm: (window as any).__loopPcm }))
  expect(evidence.draws).toBeGreaterThan(20); expect(evidence.red).toBe(0); expect(evidence.pcm.count).toBeGreaterThan(48_000); expect(evidence.pcm.peak).toBeLessThan(1e-5)
  writeEvidence('artifacts/audition-stage3/loop-privacy.json', evidence)
  const starts = evidence.starts; await page.waitForTimeout(160); expect(await page.evaluate(() => (window as any).__loopStarts)).toBe(starts)
  expect(await page.locator('audio, video').evaluateAll(elements => elements.every(element => (element as HTMLMediaElement).paused))).toBe(true)
})

test('audition restarts at A after natural video ended even when the animation clock is delayed', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, systemAudioPath: '/test-media/mix-audio.wav', edits: { systemVolume: .2 } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('button', { name: 'Audition', exact: true }).click()
  await page.getByLabel('Loop end (seconds)').fill('12'); await page.getByLabel('Loop end (seconds)').blur(); await page.getByLabel('Loop start (seconds)').fill('11.6'); await page.getByLabel('Loop start (seconds)').blur(); await page.getByLabel('Loop selection', { exact: true }).check()
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.length)).toBe(3)
  await page.evaluate(async source => {
    const w = window as any; const context = w.__audioContexts[0]; const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' })); await context.audioWorklet.addModule(url); URL.revokeObjectURL(url)
    const tap = new AudioWorkletNode(context, 'pcm-tap'); w.__tailPcm = { count: 0, peak: 0, energy: 0 }; tap.port.onmessage = event => { w.__tailPcm.count += event.data.count; w.__tailPcm.energy += event.data.energy; w.__tailPcm.peak = Math.max(w.__tailPcm.peak, event.data.peak) }; w.__audioGains[1].connect(tap); tap.connect(context.destination)
  }, pcmTapSource)
  await page.evaluate(() => {
    const w = window as any; const source = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!; w.__naturalEnds = 0; w.__tailStarts = 0
    source.addEventListener('ended', () => w.__naturalEnds++); source.addEventListener('seeked', () => { if (Math.abs(source.currentTime - 11.6) < .001) w.__tailStarts++ })
    const frame = window.requestAnimationFrame; window.requestAnimationFrame = callback => frame(now => { setTimeout(() => callback(now), 120) })
  })
  await page.getByRole('button', { name: 'Play preview', exact: true }).click(); await expect.poll(() => page.evaluate(() => (window as any).__naturalEnds), { timeout: 10_000 }).toBeGreaterThanOrEqual(2); await expect.poll(() => page.evaluate(() => (window as any).__tailStarts)).toBeGreaterThanOrEqual(3)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click(); const count = await page.evaluate(() => (window as any).__tailStarts); await page.waitForTimeout(300); expect(await page.evaluate(() => (window as any).__tailStarts)).toBe(count); expect(await page.locator('video[data-track="screen recording"]').evaluate(element => element.paused)).toBe(true)
  const retained = await page.evaluate(() => { const pcm = (window as any).__tailPcm; return { ...pcm, rms: Math.sqrt(pcm.energy / pcm.count) } }); expect(retained.count).toBeGreaterThan(48_000); expect(retained.peak).toBeCloseTo(.12, 4); expect(retained.rms).toBeGreaterThan(.1)
  writeEvidence('artifacts/audition-stage3/loop-positive-pcm.json', retained)
})

for (const action of ['pause', 'project switch']) test(`a delayed audition restart cannot resume after ${action}`, async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page)
  await page.addInitScript(() => {
    const w = window as any; w.__heldLoopCallbacks = []; const listen = HTMLMediaElement.prototype.addEventListener
    HTMLMediaElement.prototype.addEventListener = function (name: string, callback: any, options?: any) {
      if (['seeked', 'canplay', 'loadeddata'].includes(name) && typeof callback === 'function') {
        const wrapped = function (this: HTMLMediaElement, event: Event) { if (w.__holdLoopSeek && this.dataset.track === 'screen recording' && Math.abs(this.currentTime - .2) < .001) w.__heldLoopCallbacks.push(() => callback.call(this, event)); else callback.call(this, event) }
        return (listen as any).call(this, name, wrapped, options)
      }
      return (listen as any).call(this, name, callback, options)
    }
  })
  await page.goto('/'); await page.evaluate(() => { const t = (window as any).__test; t.patchProject({ cameraPath: null, systemAudioPath: '/test-media/audio.wav' }); t.addProject('after-loop', 'After loop'); t.emit('library-updated', null) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('button', { name: 'Audition', exact: true }).click(); await page.getByLabel('Loop start (seconds)').fill('.2'); await page.getByLabel('Loop start (seconds)').blur(); await page.getByLabel('Loop end (seconds)').fill('.6'); await page.getByLabel('Loop end (seconds)').blur(); await page.getByLabel('Loop selection', { exact: true }).check()
  await page.getByRole('button', { name: 'Play preview', exact: true }).click(); await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => element.currentTime)).toBeGreaterThan(.3)
  await page.evaluate(() => { (window as any).__holdLoopSeek = true }); await expect.poll(() => page.evaluate(() => (window as any).__heldLoopCallbacks.length)).toBeGreaterThan(0)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeVisible(); expect(await page.locator('audio').evaluateAll(elements => elements.every(element => element.paused))).toBe(true)
  if (action === 'pause') await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  else { await page.getByRole('button', { name: /^After loop/ }).click(); await expect(page.getByLabel('Project name')).toHaveValue('After loop') }
  await page.evaluate(() => { const w = window as any; w.__holdLoopSeek = false; for (const callback of w.__heldLoopCallbacks.splice(0)) callback() }); await page.waitForTimeout(200)
  await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeVisible(); expect(await page.locator('audio, video').evaluateAll(elements => elements.every(element => (element as HTMLMediaElement).paused))).toBe(true)
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe(action === 'pause' ? 'suspended' : 'closed')
})

test('hard cuts never draw removed red frames or play removed tone samples during delayed seeks', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page)
  await page.addInitScript(() => {
    const w = window as any; w.__removedDraws = []; w.__redDraws = 0; w.__videoDraws = 0; w.__frameTimes = {}; w.__redDetails = []; w.__copies = []
    w.__frameSources = new WeakMap(); w.__framesCreated = 0; w.__framesClosed = 0; w.__retainedSnapshots = []
    const Frame = VideoFrame; const close = Frame.prototype.close
    Frame.prototype.close = function () { if (w.__frameSources.has(this)) { w.__framesClosed++; w.__frameSources.delete(this) }; close.call(this) }
    window.VideoFrame = new Proxy(Frame, { construct(target, args) { const frame = Reflect.construct(target, args); if (args[0] instanceof HTMLVideoElement) { w.__frameSources.set(frame, args[0]); w.__framesCreated++ }; return frame } })
    const presented = HTMLVideoElement.prototype.requestVideoFrameCallback; HTMLVideoElement.prototype.requestVideoFrameCallback = function (callback: VideoFrameRequestCallback) { const source = this; return presented.call(source, (now, metadata) => { w.__frameTimes[source.dataset.track ?? 'video'] = { mediaTime: metadata.mediaTime, presentedFrames: metadata.presentedFrames, currentTime: source.currentTime }; callback(now, metadata) }) }
    const draw = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (image: CanvasImageSource, ...args: any[]) {
      const result = (draw as any).call(this, image, ...args)
      const source = image instanceof HTMLVideoElement ? image : w.__frameSources.get(image) as HTMLVideoElement | undefined
      if (source) {
        const pts = image instanceof VideoFrame ? image.timestamp / 1e6 : null
        w.__videoDraws++; w.__copies.push({ track: source.dataset.track, time: source.currentTime, pts, frame: w.__frameTimes[source.dataset.track ?? 'video'] }); if (w.__copies.length > 8) w.__copies.shift()
        const span = [[0,.9977],[2.0023,2.9977],[4.0023,4.9977]].find(([start,end]) => source.currentTime >= start && source.currentTime < end)
        if (source.seeking || !span || pts !== null && pts >= span[1]) w.__removedDraws.push({ time: source.currentTime, pts, track: source.dataset.track, seeking: source.seeking })
        if (pts !== null && span && pts < span[1]) w.__retainedSnapshots.push({ time: source.currentTime, pts, end: span[1] })
        }
      if (this.canvas.classList.contains('preview-canvas') && (source || image instanceof HTMLCanvasElement)) {
        const p = this.getImageData(Math.round(this.canvas.width * .4), Math.round(this.canvas.height * .4), 1, 1).data
        if (p[0] > 160 && p[1] < 60 && p[2] < 60) { w.__redDraws++; if (w.__redDetails.length < 30) w.__redDetails.push({ time: performance.now(), copies: structuredClone(w.__copies) }) }
      }
      return result
    } as typeof draw
    const listen = HTMLMediaElement.prototype.addEventListener
    HTMLMediaElement.prototype.addEventListener = function (name: string, callback: any, options?: any) {
      if (['seeked', 'canplay', 'loadeddata'].includes(name) && typeof callback === 'function') {
        const delayed = function (this: HTMLMediaElement, event: Event) { setTimeout(() => callback.call(this, event), 140) }
        return (listen as any).call(this, name, delayed, options)
      }
      return (listen as any).call(this, name, callback, options)
    }
  })
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ duration: 6, screenPath: '/test-media/cut-zones.mp4', cameraPath: '/test-media/cut-zones.mp4', systemAudioPath: '/test-media/cut-zones.wav', edits: { trimEnd: 6, padding: 0, cornerRadius: 0, zooms: [], showCursor: false, videoClips: [{ id: 'first', start: 0, end: .9977 }, { id: 'second', start: 2.0023, end: 2.9977 }, { id: 'third', start: 4.0023, end: 4.9977 }], music: [{ id: 'silent-kept', name: 'Removed tone fixture', path: '/test-media/cut-zones.wav', sourceDuration: 6, start: 0, sourceStart: 0, duration: 6, volume: 1, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.length)).toBe(4)
  await expect.poll(() => page.evaluate(() => (window as any).__videoDraws)).toBeGreaterThan(0)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await page.evaluate(async source => {
    const w = window as any; const context = w.__audioContexts[0] as AudioContext
    const module = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }))
    await context.audioWorklet.addModule(module); URL.revokeObjectURL(module); const tap = new AudioWorkletNode(context, 'pcm-tap'); w.__pcm = { peak: 0, count: 0, peaks: [] }; tap.port.onmessage = event => { w.__pcm.peak = Math.max(w.__pcm.peak, event.data.peak); w.__pcm.count += event.data.count; if (event.data.peak > .01 && w.__pcm.peaks.length < 40) w.__pcm.peaks.push({ peak: event.data.peak, context: context.currentTime, output: context.getOutputTimestamp(), media: [...document.querySelectorAll('video,audio')].map(element => ({ track: (element as HTMLElement).dataset.track, time: (element as HTMLMediaElement).currentTime, paused: (element as HTMLMediaElement).paused })), gates: w.__audioGains.slice(0,2).map((g: GainNode) => g.gain.value) }) }; w.__audioGains[1].connect(tap); tap.connect(context.destination)
    const oscillator = context.createOscillator(); w.__tapCalibration = oscillator; oscillator.connect(tap); oscillator.start(); await context.resume()
  }, pcmTapSource)
  await expect.poll(() => page.evaluate(() => (window as any).__pcm.peak)).toBeGreaterThan(.5)
  await page.evaluate(async () => { const w = window as any; w.__tapCalibration.stop(); w.__tapCalibration.disconnect(); await w.__audioContexts[0].suspend(); w.__tapCalibration = null })
  await page.waitForTimeout(50)
  await page.evaluate(() => { (window as any).__pcm = { peak: 0, count: 0, peaks: [] } })
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(el => (el as HTMLVideoElement).currentTime), { timeout: 10000 }).toBeGreaterThan(4.8)
  await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeVisible()
  // Under load, a paint can skip the last 33ms of every playing span. Exercise that frame explicitly.
  await seekEditor(page, 29 / 30)
  await expect.poll(() => page.evaluate(() => (window as any).__retainedSnapshots.some(
    (frame: {time: number; pts: number}) => Math.abs(frame.time - 29 / 30) < .001 && frame.pts < .9977,
  ))).toBe(true)
  const evidence = await page.evaluate(() => ({ pcm: (window as any).__pcm, removed: (window as any).__removedDraws, red: (window as any).__redDraws, redDetails: (window as any).__redDetails, draws: (window as any).__videoDraws, framesCreated: (window as any).__framesCreated, framesClosed: (window as any).__framesClosed, retainedSnapshots: (window as any).__retainedSnapshots as Array<{ time: number; pts: number; end: number }> }))
  writeEvidence(path.join(screenshotDir, 'cut-privacy-evidence.json'), evidence)
  expect(evidence.removed).toEqual([]); expect(evidence.red).toBe(0); expect(evidence.draws).toBeGreaterThan(20); expect(evidence.pcm.count).toBeGreaterThan(96_000); expect(evidence.pcm.peak).toBeLessThan(1e-5)
  expect(evidence.framesCreated).toBeGreaterThan(0); expect(evidence.framesClosed).toBe(evidence.framesCreated); expect(evidence.retainedSnapshots.some(frame => frame.time > frame.end - 1 / 30 && frame.pts < frame.end)).toBe(true)
  await page.evaluate(() => { const w = window as any; w.__videoDraws = 0; w.__test.patchProject({ cameraPath: null, systemAudioPath: '/test-media/audio.wav', edits: { music: [], systemVolume: .25 } }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts.length)).toBe(2)
  await expect.poll(() => page.evaluate(() => (window as any).__videoDraws)).toBeGreaterThan(0)
  await page.evaluate(async source => {
    const w = window as any; const context = w.__audioContexts.at(-1) as AudioContext; const module = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }))
    await context.audioWorklet.addModule(module); URL.revokeObjectURL(module); const tap = new AudioWorkletNode(context, 'pcm-tap'); w.__retainedPcm = { peak: 0, count: 0, energy: 0 }; tap.port.onmessage = event => { w.__retainedPcm.peak = Math.max(w.__retainedPcm.peak, event.data.peak); w.__retainedPcm.count += event.data.count; w.__retainedPcm.energy += event.data.energy }; w.__audioGains.at(-2).connect(tap); tap.connect(context.destination)
  }, pcmTapSource)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(el => (el as HTMLVideoElement).currentTime), { timeout: 10000 }).toBeGreaterThan(4.8)
  await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeVisible()
  const retained = await page.evaluate(() => { const pcm = (window as any).__retainedPcm; return { ...pcm, rms: Math.sqrt(pcm.energy / pcm.count) } })
  const reference = readFileSync(audioFixture); let offset = 12; let referencePeak = 0
  while (offset + 8 <= reference.length) { const length = reference.readUInt32LE(offset + 4); if (reference.toString('ascii', offset, offset + 4) === 'data') { for (let i = offset + 8; i + 4 <= Math.min(reference.length, offset + 8 + length); i += 4) referencePeak = Math.max(referencePeak, Math.abs(reference.readFloatLE(i))); break }; offset += 8 + length + (length % 2) }
  expect(referencePeak).toBeGreaterThan(.5); expect(retained.peak).toBeCloseTo(referencePeak * .25, 3); expect(retained.rms).toBeGreaterThan(referencePeak * .25 / Math.sqrt(2) * .8)
  writeEvidence(path.join(screenshotDir, 'cut-privacy-evidence.json'), { ...evidence, retained })
  await page.screenshot({ path: path.join(screenshotDir, 'video-cut-preview.png') })
})

test('a cached caption crossing a cut stays hidden and Undo restores the cached caption', async ({ page }) => {
  await mockedDesktop(page)
  await page.addInitScript(() => {
    const w = window as any; w.__captionDraws = 0; const draw = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (image: CanvasImageSource, ...args: any[]) { if (image instanceof HTMLImageElement && this.canvas.classList.contains('preview-canvas')) w.__captionDraws++; return (draw as any).call(this, image, ...args) } as typeof draw
  })
  await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ edits: { subtitles: [{ id: 'crossing-cached', start: 2, end: 7, text: 'Words from the portion that will be removed' }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const seek = async (seconds: number) => { await page.getByLabel('Playhead position in seconds').evaluate((element, time) => { (element as HTMLInputElement).value = String(time); element.dispatchEvent(new Event('input', { bubbles: true })) }, seconds) }
  await seek(2.5); await expect.poll(() => page.evaluate(() => (window as any).__captionDraws)).toBeGreaterThan(0)
  const requests = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)
  await page.getByRole('button', { name: 'Cut range...', exact: true }).click(); await page.getByLabel('Output start (seconds)').fill('3'); await page.getByLabel('Output end (seconds)').fill('6'); await page.getByRole('button', { name: 'Cut range', exact: true }).click()
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await page.evaluate(() => { (window as any).__captionDraws = 0 }); await seek(3.2)
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(el => (el as HTMLVideoElement).currentTime)).toBeCloseTo(6.2, 3)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  expect(await page.evaluate(() => (window as any).__captionDraws)).toBe(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__captionDraws)).toBeGreaterThan(0)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)).toBe(requests)
})

test('project context menu supports right-click, keyboard, dismissal and named cancellation', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  const row = page.getByRole('button', { name: 'UI verification recording' })
  await row.click({ button: 'right' })
  await expect(page.getByRole('menuitem', { name: 'Delete project...' })).toBeFocused()
  await page.screenshot({ path: path.join(screenshotDir, 'project-menu-dark.png') })
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'open_project'))).toBe(false)
  await page.keyboard.press('Escape'); await expect(row).toBeFocused(); await expect(page.getByRole('menu')).toHaveCount(0)
  await page.keyboard.press('Shift+F10'); await page.getByRole('menuitem', { name: 'Delete project...' }).click()
  const dialog = page.getByRole('dialog', { name: 'Delete project?' })
  await expect(dialog).toContainText('UI verification recording'); await expect(dialog).toContainText('Exports saved outside this folder are kept.')
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused()
  await page.screenshot({ path: path.join(screenshotDir, 'project-delete-confirmation.png') })
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click(); await expect(row).toBeFocused()
  await page.keyboard.press('ContextMenu'); await expect(page.getByRole('menu')).toBeVisible(); await page.keyboard.press('Tab'); await expect(row).toBeFocused()
  await row.dispatchEvent('contextmenu', { clientX: 1439, clientY: 899, button: 2 })
  const bounds = (await page.getByRole('menu').boundingBox())!; expect(bounds.x + bounds.width).toBeLessThanOrEqual(1432); expect(bounds.y + bounds.height).toBeLessThanOrEqual(892)
  await page.locator('.app-header h1').click(); await expect(page.getByRole('menu')).toHaveCount(0)
  await page.getByRole('button', { name: 'Switch to light theme' }).click(); await page.getByRole('button', { name: 'Project actions', exact: true }).click()
  await expect(page.getByRole('menuitem', { name: 'Delete project...' })).toBeFocused(); await page.screenshot({ path: path.join(screenshotDir, 'project-menu-light.png') }); await page.keyboard.press('Escape')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_project'))).toBe(false)
})

test('deleting an inactive project preserves the current editor and ignores a stale catalog result', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.addProject('other-project', 'Another recording'); state.emit('library-updated', null) })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByRole('button', { name: 'Another recording' })).toBeVisible()
  await page.evaluate(() => { const state = (window as any).__test; state.holdList = true; state.emit('library-updated', null) })
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishList)).toBe('function')
  await page.getByRole('button', { name: 'Another recording' }).click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click()
  await page.getByRole('button', { name: 'Move to Recycle Bin', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Another recording' })).toHaveCount(0)
  await expect(page.getByLabel('Project name')).toHaveValue('UI verification recording')
  await page.evaluate(() => { const state = (window as any).__test; state.holdList = false; state.finishList() })
  await expect(page.getByRole('button', { name: 'Another recording' })).toHaveCount(0)
  expect(await page.evaluate(() => (window as any).__test.calls.find((call: any) => call.name === 'delete_project').args)).toEqual({ id: 'other-project' })
})

test('deleting the last current project waits for saving and releases media without autosave resurrection', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await page.getByRole('button', { name: 'UI verification recording' }).click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click(); await page.getByRole('button', { name: 'Move to Recycle Bin', exact: true }).click()
  await expect(page.getByRole('dialog', { name: 'Delete project?' }).getByRole('button', { name: 'Cancel' })).toBeDisabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_project'))).toBe(false)
  await page.evaluate(() => { const state = (window as any).__test; state.holdSave = false; state.finishSave() })
  await expect(page.getByRole('heading', { name: 'Record your screen' })).toBeVisible()
  await expect(page.getByLabel('Project name')).toHaveCount(0); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toHaveCount(0)
  expect(await page.evaluate(() => (window as any).__test.mediaAtDelete)).toEqual([])
  expect(await page.evaluate(() => (window as any).__test.saved.edits.aspectRatio)).toBe('1:1')
  await page.waitForTimeout(1000)
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name))
  expect(calls.slice(calls.indexOf('delete_project') + 1)).not.toContain('save_project_edits')
  await page.getByRole('button', { name: 'Refresh', exact: true }).click(); await expect(page.getByRole('button', { name: 'UI verification recording' })).toHaveCount(0)
  await page.evaluate(() => (window as any).__test.restoreProject('ui-test'))
  await page.getByRole('button', { name: 'Refresh', exact: true }).click(); await expect(page.getByRole('button', { name: 'UI verification recording' })).toBeVisible()
})

test('deleting the current project opens another recording with clean history', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.addProject('next-project', 'Next recording'); state.emit('library-updated', null) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByLabel('Aspect ratio').selectOption('1:1')
  await page.getByRole('button', { name: 'UI verification recording' }).click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click(); await page.getByRole('button', { name: 'Move to Recycle Bin', exact: true }).click()
  await expect(page.getByLabel('Project name')).toHaveValue('Next recording'); await expect(page.getByLabel('Aspect ratio')).toHaveValue('source'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'UI verification recording' })).toHaveCount(0)
})

test('save and recycle failures preserve project edits and allow an explicit delete retry', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.saveError = 'Disk is not writable.' })
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await page.getByLabel('Project name').fill('Unsaved project title')
  await page.getByLabel('Project name').blur(); await expect(page.getByRole('alert')).toContainText('Disk is not writable.')
  await page.getByRole('button', { name: 'UI verification recording' }).click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click(); await page.getByRole('button', { name: 'Move to Recycle Bin', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Delete project?' })
  await expect(dialog.locator('.delete-project-name')).toHaveText('Unsaved project title')
  await expect(dialog.getByRole('alert')).toContainText('Disk is not writable.')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_project'))).toBe(false)
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('1:1')
  await page.evaluate(() => { const state = (window as any).__test; state.saveError = null; state.deleteError = 'Recycle Bin is unavailable.' })
  await dialog.getByRole('button', { name: 'Retry delete', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('Recycle Bin is unavailable.')
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('1:1'); await expect(page.locator('.preview-host')).toBeVisible()
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click(); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: 'Unsaved project title' }).click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click()
  await page.evaluate(() => { (window as any).__test.deleteError = null }); await page.getByRole('button', { name: 'Move to Recycle Bin', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Record your screen' })).toBeVisible()
})

test('project deletion cannot invoke while an export is running', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  const row = page.getByRole('button', { name: 'UI verification recording' })
  await row.click({ button: 'right' }); await expect(page.getByRole('menu')).toBeVisible()
  await page.evaluate(() => (window as any).__test.emit('export-progress', { state: 'running', progress: .4, outputPath: 'D:\\export.mp4', error: null }))
  await expect(page.getByRole('menu')).toHaveCount(0)
  await row.dispatchEvent('contextmenu', { button: 2 }); await expect(page.getByRole('menu')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Project actions', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('export-progress', { state: 'cancelled', progress: .4, outputPath: 'D:\\export.mp4', error: null }))
  await row.click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Delete project...' }).click()
  await page.evaluate(() => (window as any).__test.emit('export-progress', { state: 'running', progress: .4, outputPath: 'D:\\export.mp4', error: null }))
  await expect(page.getByRole('button', { name: 'Move to Recycle Bin', exact: true })).toBeDisabled()
  await page.getByRole('dialog', { name: 'Delete project?' }).locator('form').dispatchEvent('submit')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'delete_project'))).toBe(false)
})

test('direct camera and mask gestures update properties with one undo, Escape and busy-state rollback', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { padding: 0, zooms: [], camera: { enabled: true, x: .65, y: .55, width: .2, shape: 'rectangle', mirror: false }, redactions: [{ id: 'drag-mask', start: 0, end: 12, x: .2, y: .2, width: .2, height: .2, mode: 'solid', strength: 16 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => (element as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(2)
  const canvas = page.getByLabel('Edited video preview'); const box = (await canvas.boundingBox())!
  await page.getByRole('tab', { name: 'Canvas', exact: true }).focus()
  await page.mouse.move(box.x + box.width * .75, box.y + box.height * .65); await page.mouse.down(); await page.mouse.move(box.x + box.width * .65, box.y + box.height * .55, { steps: 12 }); await page.mouse.up()
  await expect(page.getByRole('tab', { name: 'Camera', exact: true })).toHaveAttribute('aria-selected', 'true')
  await expect(page.locator('#camera-x')).toHaveValue('0.55'); await expect(page.locator('#camera-y')).toHaveValue('0.45')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.locator('#camera-x')).toHaveValue('0.65'); await expect(page.locator('#camera-y')).toHaveValue('0.55'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.mouse.move(box.x + box.width * .3, box.y + box.height * .3); await page.mouse.down(); await page.mouse.move(box.x + box.width * .4, box.y + box.height * .38, { steps: 12 }); await page.mouse.up()
  await expect(page.getByRole('tab', { name: 'Masks', exact: true })).toHaveAttribute('aria-selected', 'true')
  expect(Number(await page.getByLabel('Region left %', { exact: true }).inputValue())).toBeCloseTo(30, 1)
  expect(Number(await page.getByLabel('Region top %', { exact: true }).inputValue())).toBeCloseTo(28, 1)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('drag-mask')
  await expect(page.getByLabel('Region left %', { exact: true })).toHaveValue('20.00'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  const handle = page.getByRole('button', { name: 'Move mask', exact: true }); const handleBox = (await handle.boundingBox())!
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2); await page.mouse.down(); await page.mouse.move(handleBox.x + handleBox.width / 2 + 30, handleBox.y + handleBox.height / 2 + 20, { steps: 4 }); await page.keyboard.press('Escape'); await page.mouse.up()
  await expect(page.getByLabel('Region left %', { exact: true })).toHaveValue('20.00'); await expect(page.getByLabel('Region top %', { exact: true })).toHaveValue('20.00'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  const restored = (await handle.boundingBox())!
  await page.mouse.move(restored.x + restored.width / 2, restored.y + restored.height / 2); await page.mouse.down(); await page.mouse.move(restored.x + restored.width / 2 + 25, restored.y + restored.height / 2 + 15, { steps: 4 })
  await page.evaluate(() => (window as any).__test.emit('export-progress', { state: 'running', progress: .4, outputPath: 'D:\\export.mp4', error: null }))
  await expect(page.getByLabel('Region left %', { exact: true })).toHaveValue('20.00'); await expect(page.getByLabel('Region top %', { exact: true })).toHaveValue('20.00')
  await page.mouse.up()
  await page.evaluate(() => (window as any).__test.emit('export-progress', { state: 'cancelled', progress: .4, outputPath: 'D:\\export.mp4', error: null }))
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
})

test('recording setup previews only on request, releases before capture, and remembers validated user choices', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 1600, height: 900 }); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { camera: { enabled: false, x: .7, y: .7, width: .2, shape: 'circle', mirror: true } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('button', { name: 'No camera recorded', exact: true }).click()
  await expect(page.getByRole('tabpanel', { name: 'Camera' })).toContainText('This project contains no webcam video')
  await page.getByRole('button', { name: 'New recording with camera', exact: true }).click()
  await expect(page.getByLabel('Webcam', { exact: false }).first()).toHaveValue('cam-test')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_camera_preview'))).toBe(false)
  await page.getByLabel('Microphone', { exact: false }).filter({ has: page.locator('option') }).selectOption('mic-test')
  await page.getByRole('dialog', { name: 'New recording' }).getByLabel('Frame rate', { exact: true }).selectOption('60')
  await page.getByLabel('Record computer audio').uncheck()
  await page.getByRole('button', { name: 'Enable preview', exact: true }).click()
  await expect(page.getByRole('img', { name: 'Live preview of the selected camera' })).toBeVisible()
  await page.screenshot({ path: path.join(screenshotDir, 'record-setup-1600-dark.png') })
  await page.evaluate(() => { (window as any).__test.holdPreviewStop = true })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishPreviewStop)).toBe('function')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
  await page.evaluate(() => { const state = (window as any).__test; state.holdPreviewStop = false; state.finishPreviewStop() })
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeVisible()
  const calls = await page.evaluate(() => (window as any).__test.calls)
  expect(calls.findIndex((call: any) => call.name === 'stop_camera_preview')).toBeLessThan(calls.findIndex((call: any) => call.name === 'start_recording'))
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await page.reload(); await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  await expect(page.locator('#webcam')).toHaveValue('cam-test'); await expect(page.locator('#microphone')).toHaveValue('mic-test'); await expect(page.locator('#record-fps')).toHaveValue('60'); await expect(page.getByLabel('Record computer audio')).not.toBeChecked()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_camera_preview'))).toBe(false)
  await page.getByRole('dialog', { name: 'New recording' }).getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.evaluate(() => localStorage.setItem('screenrecord-recording-options', JSON.stringify({ cameraId: 'unplugged-camera', microphoneId: 'unplugged-mic', fps: 30, systemAudio: true })))
  await page.reload(); await page.getByRole('button', { name: 'New recording', exact: true }).first().click()
  await expect(page.locator('#webcam option:checked')).toHaveText('Off - record without camera'); await expect(page.locator('#microphone option:checked')).toHaveText('Off')
  await expect(page.getByRole('button', { name: 'Enable preview', exact: true })).toBeDisabled()
})

test('Cancel and Escape release camera preview before closing recording setup', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  for (const action of ['cancel', 'escape']) {
    await page.getByRole('button', { name: 'New recording', exact: true }).first().click(); await page.locator('#webcam').selectOption('cam-test')
    await page.getByRole('button', { name: 'Enable preview', exact: true }).click(); await expect(page.getByRole('img', { name: 'Live preview of the selected camera' })).toBeVisible()
    await page.evaluate(() => { const state = (window as any).__test; state.holdPreviewStop = true; state.finishPreviewStop = null })
    if (action === 'cancel') await page.getByRole('dialog', { name: 'New recording' }).getByRole('button', { name: 'Cancel', exact: true }).click()
    else await page.keyboard.press('Escape')
    await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishPreviewStop)).toBe('function')
    await expect(page.getByRole('dialog', { name: 'New recording' })).toBeVisible()
    await page.evaluate(() => { const state = (window as any).__test; state.holdPreviewStop = false; state.finishPreviewStop() })
    await expect(page.getByRole('dialog', { name: 'New recording' })).toBeHidden()
  }
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_recording'))).toBe(false)
})

async function openMultitrackTimeline(page: Page) {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    microphonePath: '/test-media/audio.wav',
    systemAudioPath: '/test-media/mix-audio.wav',
    edits: {
      subtitles: [{ id: 'caption', start: 1, end: 4, text: 'Timeline scrolling' }],
      redactions: Array.from({ length: 5 }, (_, index) => ({
        id: 'mask-' + index, start: 0, end: 6, x: .1, y: .1, width: .2, height: .2, mode: 'solid', strength: 12,
      })),
    },
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const projects = page.getByRole('button', { name: 'Projects', exact: true })
  if (await projects.getAttribute('aria-expanded') === 'true') await projects.click()
  await expect(page.locator('#playhead')).toBeVisible()
}

test('timeline wheel reaches the last track from both the toolbar and the lanes in a short window', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 640 })
  await openMultitrackTimeline(page)
  const timeline = page.getByRole('region', { name: 'Video timeline' })
  const scroll = timeline.locator('.timeline-scroll')
  const scrollTop = () => scroll.evaluate(el => el.scrollTop)
  const playhead = timeline.locator('#playhead')
  const startTime = await playhead.inputValue()
  const lastClip = timeline.getByRole('button', { name: /^Edit masks 5:/ })
  await expect(lastClip).not.toBeInViewport()
  await timeline.getByRole('button', { name: 'Split', exact: true }).hover()
  await page.mouse.wheel(0, 820)
  await expect(lastClip).toBeInViewport({ ratio: .9 })
  expect(await scrollTop()).toBeGreaterThan(0)
  await lastClip.hover()
  await page.mouse.wheel(0, -820)
  await expect.poll(scrollTop).toBe(0)
  const trackRegion = timeline.getByRole('region', { name: 'Timeline tracks' })
  await trackRegion.focus()
  await trackRegion.press('PageDown')
  await expect.poll(scrollTop).toBeGreaterThan(0)
  await trackRegion.press('End')
  await expect(lastClip).toBeInViewport({ ratio: .9 })
  await trackRegion.press('Home')
  await expect.poll(scrollTop).toBe(0)

  await timeline.getByRole('button', { name: 'Volume keyframes', exact: true }).click()
  await timeline.getByRole('button', { name: 'Audition', exact: true }).click()
  const bounds = (await scroll.boundingBox())!
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(641)
  expect(bounds.height).toBeGreaterThan(62)
  await page.mouse.move(bounds.x + bounds.width * .7, bounds.y + bounds.height - 15)
  await page.mouse.wheel(0, 1400)
  await expect(lastClip).toBeInViewport({ ratio: .9 })
  await expect(playhead).toHaveValue(startTime)
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  expect(await page.evaluate(() => window.scrollY)).toBe(0)
})

test('timeline zoom anchors the time under the pointer and horizontal navigation keeps the playhead in place', async ({ page }) => {
  await openMultitrackTimeline(page)
  const timeline = page.getByRole('region', { name: 'Video timeline' })
  const tracks = timeline.locator('.timeline-scroll')
  const plane = timeline.locator('.timeline-plane')
  const scale = timeline.locator('#timeline-scale')
  const playhead = timeline.locator('#playhead')
  const initialTime = await playhead.inputValue()
  const initialPlane = (await plane.boundingBox())!
  const x = initialPlane.x + initialPlane.width * .64
  const fractionAtPointer = () => plane.evaluate((el, x) => {
    const rect = el.getBoundingClientRect()
    return (x - rect.left) / rect.width
  }, x)
  await page.mouse.move(x, initialPlane.y + 6)
  await page.keyboard.down('Control')
  try {
    for (let i = 0; i < 3; i++) await page.mouse.wheel(0, -1)
  } finally { await page.keyboard.up('Control') }
  await expect(scale).toHaveValue('1')
  await page.keyboard.down('Control')
  try { await page.mouse.wheel(0, -120) } finally { await page.keyboard.up('Control') }
  await expect(scale).toHaveValue('2')
  await expect.poll(fractionAtPointer).toBeCloseTo(.64, 2)
  const readLeft = () => tracks.evaluate(el => el.scrollLeft)
  const firstLeft = await readLeft()
  expect(firstLeft).toBeGreaterThan(0)
  await page.keyboard.down('Shift')
  try { await page.mouse.wheel(0, 95) } finally { await page.keyboard.up('Shift') }
  await expect.poll(readLeft).toBeGreaterThan(firstLeft)
  const movedLeft = await readLeft()
  await page.mouse.wheel(-65, 0)
  await expect.poll(readLeft).toBeLessThan(movedLeft)

  const zoomIn = timeline.getByRole('button', { name: 'Zoom in on timeline', exact: true })
  const zoomOut = timeline.getByRole('button', { name: 'Zoom out on timeline', exact: true })
  await zoomIn.click()
  await expect(scale).toHaveValue('4')
  await zoomIn.focus()
  await page.keyboard.press('Enter')
  await expect(scale).toHaveValue('8')
  await expect(zoomIn).toBeDisabled()
  await zoomOut.click()
  await expect(scale).toHaveValue('4')
  await page.mouse.move(x, initialPlane.y + 6)
  const beforeZoomOut = await fractionAtPointer()
  await page.keyboard.down('Control')
  try { await page.mouse.wheel(0, 120) } finally { await page.keyboard.up('Control') }
  await expect(scale).toHaveValue('2')
  await expect.poll(fractionAtPointer).toBeCloseTo(beforeZoomOut, 2)
  await scale.selectOption('1')
  await expect(zoomOut).toBeDisabled()
  await expect.poll(readLeft).toBe(0)
  await expect(playhead).toHaveValue(initialTime)
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
})

test('timeline navigation leaves a dragged volume point and its rollback intact', async ({ page }) => {
  await openMultitrackTimeline(page)
  await seekEditor(page, 4)
  const timeline = page.getByRole('region', { name: 'Video timeline' })
  await timeline.getByRole('button', { name: 'Volume keyframes', exact: true }).click()
  await timeline.getByRole('button', { name: 'Add at playhead' }).click()
  const point = timeline.getByRole('button', { name: 'Microphone volume point at 4 seconds, 100%', exact: true })
  await point.hover()
  const box = (await point.boundingBox())!
  const tracks = timeline.getByRole('region', { name: 'Timeline tracks' })
  const top = await tracks.evaluate(el => el.scrollTop)
  const scale = timeline.locator('#timeline-scale')
  await page.mouse.down()
  try {
    await page.mouse.move(box.x + 48, box.y + 13, { steps: 4 })
    await expect(point).toHaveCount(0)
    await page.keyboard.down('Control')
    try { await page.mouse.wheel(0, -120) } finally { await page.keyboard.up('Control') }
    await page.mouse.wheel(0, 200)
    await expect(scale).toHaveValue('1')
    expect(await tracks.evaluate(el => el.scrollTop)).toBe(top)
    await page.keyboard.press('Escape')
  } finally { await page.mouse.up() }
  await expect(point).toHaveCount(1)
  await timeline.getByRole('button', { name: 'Zoom in on timeline' }).click()
  await expect(scale).toHaveValue('2')
})

test('screen and zoom timeline handles edit directly with single-step undo and a concrete export summary', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('button', { name: 'Trim start of screen recording', exact: true }).focus(); await page.keyboard.press('Shift+ArrowRight')
  await expect(page.getByLabel('Start (seconds)', { exact: true })).toHaveValue('1')
  await page.getByRole('button', { name: 'Trim end of screen recording', exact: true }).focus(); await page.keyboard.press('Shift+ArrowLeft')
  await expect(page.getByLabel('End (seconds)', { exact: true })).toHaveValue('11')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.getByLabel('End (seconds)', { exact: true })).toHaveValue('12')
  const clip = page.getByRole('button', { name: /^Edit zoom 1,/ }); const box = (await clip.boundingBox())!; const width = await page.locator('.timeline-plane').evaluate(element => element.getBoundingClientRect().width)
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + width / 11, box.y + box.height / 2, { steps: 12 }); await page.mouse.up()
  await expect(page.getByLabel('Start (seconds)', { exact: true })).toHaveValue('3.00'); await expect(page.getByLabel('End (seconds)', { exact: true })).toHaveValue('6.00')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await clip.click(); await expect(page.getByLabel('Start (seconds)', { exact: true })).toHaveValue('2.00')
  await page.getByRole('button', { name: 'Export MP4', exact: true }).click()
  await expect(page.locator('.export-summary')).toContainText('1920 x 1080'); await expect(page.locator('.export-summary')).toContainText('0:11.0')
  await page.getByLabel('Resolution', { exact: true }).selectOption('1440p'); await expect(page.locator('.export-summary')).toContainText('2560 x 1440')
})

test('populated editor preserves preview space, reachable tools and measured contrast across practical viewport sizes', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ version: 3, systemAudioPath: '/test-media/audio.wav', edits: { aspectRatio: '16:9', crop: null, music: [{ id: 'layout-music', name: 'Background music', path: '/test-media/audio.wav', sourceDuration: 12, start: 0, sourceStart: 0, duration: 6, volume: .4, fadeIn: 1, fadeOut: 2 }], subtitles: [{ id: 'layout-cue', start: 7, end: 10, text: 'Review this step before continuing.' }], redactions: [{ id: 'layout-mask', start: 0, end: 3, x: .12, y: .2, width: .2, height: .12, mode: 'solid', strength: 16 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('button', { name: 'Projects', exact: true }).click()
  await page.getByRole('button', { name: /^Edit music 1:/ }).click(); await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  const measurements: any[] = []
  for (const [width, height] of [[900, 640], [1280, 720], [1600, 900], [1920, 1080], [2560, 1440]]) {
    await page.setViewportSize({ width, height })
    for (const theme of ['dark', 'light']) {
      if (await page.locator('html').getAttribute('data-theme') !== theme) await page.getByRole('button', { name: /Switch to/ }).click()
      await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && document.documentElement.scrollHeight <= innerHeight)).toBe(true)
      await page.getByRole('tab', { name: 'Canvas', exact: true }).focus(); await page.keyboard.press('End'); await expect(page.getByRole('tab', { name: 'Masks', exact: true })).toBeFocused()
      const tool = (await page.getByRole('tab', { name: 'Masks', exact: true }).boundingBox())!; expect(tool.y + tool.height).toBeLessThanOrEqual(height)
      await page.getByRole('tab', { name: 'Music', exact: true }).click(); await page.getByRole('button', { name: 'Delete music clip', exact: true }).scrollIntoViewIfNeeded()
      const preview = (await page.locator('.preview-host').boundingBox())!; const timeline = (await page.getByRole('region', { name: 'Video timeline' }).boundingBox())!
      expect(preview.height).toBeGreaterThan(height >= 900 ? timeline.height * 1.5 : 180)
      expect(timeline.height).toBeLessThanOrEqual(260); expect(timeline.y + timeline.height).toBeLessThanOrEqual(height)
      expect(await page.getByRole('tab', { name: 'Music', exact: true }).evaluate(element => getComputedStyle(element).fontSize)).toBe('13px')
      const contrast = await page.evaluate(() => {
        const numbers = (color: string) => (color.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number)
        const lum = (color: string) => numbers(color).map(n => { const c = n / 255; return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4 }).reduce((sum, value, i) => sum + value * [.2126, .7152, .0722][i], 0)
        const ratio = (a: string, b: string) => (Math.max(lum(a), lum(b)) + .05) / (Math.min(lum(a), lum(b)) + .05)
        const input = getComputedStyle(document.querySelector('#music-name')!); const tab = getComputedStyle(document.querySelector('#tab-music')!); const rail = getComputedStyle(document.querySelector('.inspector-tabs')!); const action = getComputedStyle(document.querySelector('.header-actions .primary-button')!)
        return { inputText: ratio(input.color, input.backgroundColor), inputBoundary: ratio(input.borderTopColor, input.backgroundColor), selectedText: ratio(tab.color, tab.backgroundColor), selectedBoundary: ratio(tab.borderTopColor, rail.backgroundColor), actionText: ratio(action.color, action.backgroundColor), focus: ratio(tab.outlineColor, rail.backgroundColor) }
      })
      expect(contrast.inputText).toBeGreaterThanOrEqual(4.5); expect(contrast.inputBoundary).toBeGreaterThanOrEqual(3); expect(contrast.selectedText).toBeGreaterThanOrEqual(4.5); expect(contrast.selectedBoundary).toBeGreaterThanOrEqual(3); expect(contrast.actionText).toBeGreaterThanOrEqual(4.5); expect(contrast.focus).toBeGreaterThanOrEqual(3)
      measurements.push({ width, height, theme, previewHeight: preview.height, timelineHeight: timeline.height, contrast })
      await page.locator('.inspector-content').evaluate(element => { element.scrollTop = 0 })
      await page.screenshot({ path: path.join(screenshotDir, `editor-${width}x${height}-${theme}.png`) })
    }
  }
  writeEvidence(path.join(screenshotDir, 'layout-contrast.json'), measurements)
})

test('music import, source trim, fades, direct dragging and undo persist through the small edit DTO', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('tab', { name: 'Music', exact: true }).click(); await page.getByRole('button', { name: 'Import music' }).click()
  await expect(page.getByLabel('Clip name')).toHaveValue('Music fixture')
  await page.getByLabel('Audio in point (s)').fill('2'); await page.getByLabel('Audio in point (s)').blur()
  await page.getByLabel('Fade in (s)').fill('1'); await page.getByLabel('Fade in (s)').blur()
  await page.getByLabel('Fade out (s)').fill('2'); await page.getByLabel('Fade out (s)').blur()
  const body = page.getByRole('button', { name: /^Edit music 1:/ }); await body.scrollIntoViewIfNeeded()
  const box = (await body.boundingBox())!; const width = await page.locator('.timeline-plane').evaluate(element => element.getBoundingClientRect().width)
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + width / 6, box.y + box.height / 2, { steps: 12 }); await page.mouse.up()
  await expect(page.getByLabel('Timeline start (s)')).toHaveValue('2.000')
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await body.click(); await expect(page.getByLabel('Timeline start (s)')).toHaveValue('0.000')
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await body.click()
  await expect(page.getByLabel('Timeline start (s)')).toHaveValue('2.000')
  const handle = page.getByRole('button', { name: 'Trim start of music 1', exact: true }); await handle.focus(); await page.keyboard.press('Shift+ArrowRight')
  await expect(page.getByLabel('Timeline start (s)')).toHaveValue('3.000'); await expect(page.getByLabel('Audio in point (s)')).toHaveValue('3.000')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  const saved = await page.evaluate(() => (window as any).__test.saved)
  expect(saved.edits.music[0]).toMatchObject({ start: 3, sourceStart: 3, duration: 5, fadeIn: 1, fadeOut: 2 })
  await page.screenshot({ path: path.join(screenshotDir, 'editor-music.png') })
  await page.getByRole('button', { name: 'Delete music clip' }).click(); await expect(body).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(body).toHaveCount(1)
})

test('subtitles are editable, searchable, exported, generated undoably and cancelled without retaining a key', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ version: 3, microphonePath: '/test-media/audio.wav' }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click()
  await page.getByRole('button', { name: 'Add subtitle', exact: true }).click()
  await page.getByLabel('Subtitle text', { exact: true }).fill('Halo, dunia!\nPeriksa data.'); await page.getByLabel('Subtitle text', { exact: true }).blur()
  await page.getByRole('button', { name: 'Export SRT', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'export_subtitles'))).toBe(true)
  await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click()
  await page.getByLabel('Recognition provider').selectOption('cloud')
  await expect(page.getByRole('dialog')).toContainText('uploads the selected audio')
  await page.getByLabel('Deepgram API key').fill('test-key-not-real'); await page.getByRole('button', { name: 'Upload audio and generate' }).click()
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  await page.evaluate(() => { const state = (window as any).__test; state.emit('transcription-progress', { state: 'completed', projectId: 'ui-test', jobId: state.calls.find((call: any) => call.name === 'start_transcription').args.jobId, progress: 1, error: null, cues: [{ id: 'generated-cue', start: .5, end: 2, text: 'Hasil transkripsi.' }, { id: 'generated-cue-2', start: 2, end: 2.01, text: 'Ya' }] }) })
  await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Hasil transkripsi.')
  await page.getByRole('button', { name: 'Next cue', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Ya')
  await page.getByLabel('Search subtitle text').fill('transkripsi'); await expect(page.locator('#subtitle-selection option')).toHaveCount(2)
  await page.getByLabel('Search subtitle text').fill(''); await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await page.getByLabel('Subtitle cue').selectOption({ label: '0:00.0 Halo, dunia! Periksa data.' }).catch(async () => { await page.getByLabel('Subtitle cue').selectOption({ index: 1 }) })
  await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Halo, dunia!\nPeriksa data.')
  await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click(); await page.getByLabel('Recognition provider').selectOption('cloud'); await expect(page.getByLabel('Deepgram API key')).toHaveValue('')
  await page.getByLabel('Deepgram API key').fill('test-key-not-real'); await page.getByRole('button', { name: 'Upload audio and generate' }).click(); await page.getByRole('button', { name: 'Cancel transcription', exact: true }).click()
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', ''); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Halo, dunia!\nPeriksa data.')
})

test('drawn solid masks stay in source coordinates under crop and can be timed and undone', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { crop: { x: .25, y: .1, width: .5, height: .8 }, zooms: [], padding: 0, cornerRadius: 0, showCursor: false } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByRole('button', { name: 'Draw on preview', exact: true }).click()
  await page.keyboard.press('Escape'); await expect(page.getByRole('button', { name: 'Draw on preview', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Draw on preview', exact: true }).click()
  const box = (await page.locator('canvas').boundingBox())!; const ratio = 480 / 432; const width = Math.min(box.width, box.height * ratio); const height = width / ratio
  const left = box.x + (box.width - width) / 2; const top = box.y + (box.height - height) / 2
  await page.mouse.move(left + width * .2, top + height * .3); await page.mouse.down(); await page.mouse.move(left + width * .6, top + height * .6, { steps: 8 }); await page.mouse.up()
  await expect(page.getByLabel('Effect', { exact: true })).toHaveValue('solid')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  const region = await page.evaluate(() => (window as any).__test.saved.edits.redactions[0])
  expect(region.x).toBeCloseTo(.35, 2); expect(region.y).toBeCloseTo(.34, 2); expect(region.width).toBeCloseTo(.2, 2); expect(region.height).toBeCloseTo(.24, 2)
  const pixel = await page.locator('canvas').evaluate(canvas => { const c = canvas as HTMLCanvasElement; return [...c.getContext('2d')!.getImageData(c.width * .4, c.height * .45, 1, 1).data] })
  expect(pixel).toEqual([0, 0, 0, 255])
  await page.screenshot({ path: path.join(screenshotDir, 'editor-masks.png') })
  await page.getByLabel('Region end (s)').fill('1'); await page.getByLabel('Region end (s)').blur()
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region').selectOption({ index: 1 })
  await expect(page.getByLabel('Region end (s)')).toHaveValue('3.000')
  await page.getByRole('button', { name: 'Delete region', exact: true }).click(); await expect(page.getByRole('button', { name: /^Edit masks/ })).toHaveCount(0)
})

test('undo during a delayed autosave keeps the newer state and keyboard redo remains available', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('source')
  await page.evaluate(() => { const state = (window as any).__test; state.holdSave = false; state.finishSave() })
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('source')
  await page.getByRole('button', { name: 'Redo', exact: true }).focus(); await page.keyboard.press('Control+Shift+Z')
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('1:1')
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved.edits.aspectRatio)).toBe('1:1')
})

test('a long input gesture serializes once and saves only its committed result', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { padding: 60, subtitles: Array.from({ length: 1000 }, (_, i) => ({ id: `caption-${i}`, start: i / 100, end: (i + 1) / 100, text: `Synthetic caption ${i}` })), subtitleStyle: { enabled: false, fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: false, highlightColor: '#E9B451' } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.locator('#padding').focus()
  await page.evaluate(async () => {
    const w = window as any; w.__editSerializations = 0; w.__test.calls = []
    const stringify = JSON.stringify
    JSON.stringify = function (value: any, ...args: any[]) { if (value && (value.edits || Object.hasOwn(value, 'trimEnd'))) w.__editSerializations++; return (stringify as any)(value, ...args) } as typeof stringify
    const input = document.querySelector<HTMLInputElement>('#padding')!
    for (let i = 0; i < 60; i++) { input.value = String(40 + i * 2); input.dispatchEvent(new Event('input', { bubbles: true })); await new Promise(requestAnimationFrame) }
  })
  await page.waitForTimeout(1100)
  expect(await page.evaluate(() => (window as any).__editSerializations)).toBe(0)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'save_project_edits'))).toEqual([])
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible()
  await page.locator('#padding').blur()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.padding)).toBe(158)
  expect(await page.evaluate(() => (window as any).__editSerializations)).toBeLessThanOrEqual(8)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.locator('#padding')).toHaveValue('60')
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await expect(page.locator('#padding')).toHaveValue('158')
})

test('held preview gestures do not autosave or undo earlier edits and Escape preserves Redo', async ({ page }) => {
  await openTrackedMask(page, true); await seekEditor(page, 4.5)
  const left = await page.getByLabel('Region left %', { exact: true }).inputValue()
  await page.getByLabel('Region left %', { exact: true }).fill(String(Number(left) + 5)); await page.getByLabel('Region left %', { exact: true }).blur()
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked')
  await expect(page.getByRole('button', { name: 'Redo', exact: true })).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.calls = [] })
  const handle = page.getByRole('button', { name: 'Move mask', exact: true }); const box = (await handle.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2 + 15, { steps: 20 })
  await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible()
  await page.waitForTimeout(1100); await page.keyboard.press('Control+z'); await page.keyboard.press('Control+s')
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'save_project_edits'))).toBe(false)
  await page.keyboard.press('Escape'); await page.mouse.up()
  await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible(); await expect(page.getByLabel('Region left %', { exact: true })).toHaveValue(left)
  await expect(page.getByRole('button', { name: 'Redo', exact: true })).toBeEnabled(); await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'save_project_edits'))).toBe(false)
})

test('timeline gesture Escape and native close roll back before any save or shortcut mutation', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { music: [{ id: 'timeline-gesture', name: 'Gesture music', path: '/test-media/audio.wav', sourceDuration: 12, start: 2, sourceStart: 0, duration: 4, volume: .5, fadeIn: 0, fadeOut: 0 }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const clip = page.getByRole('button', { name: /^Edit music 1:/ }); await clip.click()
  const begin = async () => { const box = (await clip.boundingBox())!; await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2, { steps: 10 }) }
  await page.evaluate(() => { (window as any).__test.calls = [] }); await begin(); await page.waitForTimeout(1100)
  await page.keyboard.press('Control+z'); await page.keyboard.press('s'); await page.keyboard.press('Delete')
  await expect(page.getByRole('button', { name: /^Select video clip/ })).toHaveCount(1)
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'save_project_edits'))).toBe(false)
  await page.keyboard.press('Escape'); await page.mouse.up(); await expect(page.getByLabel('Timeline start (s)')).toHaveValue('2.000'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await begin(); await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null)); await page.mouse.up()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(true)
  await expect(page.getByLabel('Timeline start (s)')).toHaveValue('2.000')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'save_project_edits'))).toBe(false)
})

test('a delayed save ACK preserves an active keyboard edit and Undo to the saved state becomes clean', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/'); await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  const padding = page.locator('#padding'); await padding.focus(); await padding.press('ArrowRight'); await padding.press('Tab')
  await page.getByRole('button', { name: 'Save', exact: true }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await padding.focus(); await padding.press('ArrowRight'); await expect(padding).toHaveValue('46')
  await page.evaluate(() => { const state = (window as any).__test; state.holdSave = false; state.finishSave() })
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.padding)).toBe(44)
  await expect(padding).toHaveValue('46'); await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible()
  await padding.press('Tab'); await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(padding).toHaveValue('44'); await expect(page.getByText('Saved', { exact: true })).toBeVisible(); await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'Redo', exact: true })).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'save_project_edits').length)).toBe(1)
})

test('queued edit commits cannot enter the next project history', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.addProject('next-transaction', 'Next transaction'); state.emit('library-updated', null) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByRole('button', { name: /^Next transaction/ })).toBeVisible()
  await page.evaluate(() => {
    const input = document.querySelector<HTMLInputElement>('#padding')!; input.value = '62'; input.dispatchEvent(new Event('input', { bubbles: true }))
    ;(document.querySelector('#project-name-next-transaction')!.closest('button') as HTMLButtonElement).click()
  })
  await expect(page.getByLabel('Project name')).toHaveValue('Next transaction'); await expect(page.locator('#padding')).toHaveValue('42'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.padding)).toBe(62)
})

test('subtitle raster requests are active-first, serial and retry superseded rendering', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.holdSubtitle = true; state.cancelSubtitleOnce = true; state.patchProject({ edits: { subtitles: [{ id: 'active', start: 0, end: 2, text: 'Active cue' }, { id: 'next', start: 3, end: 5, text: 'Next cue' }] } }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)).toBe(1)
  await page.waitForTimeout(350)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').map((call: any) => call.args.cue.id))).toEqual(['active'])
  await page.evaluate(() => { const state = (window as any).__test; state.holdSubtitle = false; state.finishSubtitle() })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').map((call: any) => call.args.cue.id))).toEqual(['active', 'active', 'next'])
  await expect(page.getByText('Rendering subtitle preview...', { exact: true })).toBeHidden()
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('old transcription results cannot replace a new job and cancellation waits for its registration', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const state = (window as any).__test; state.holdTranscription = true; state.patchProject({ microphonePath: '/test-media/audio.wav', edits: { subtitles: [{ id: 'original', start: 0, end: 2, text: 'Keep this cue' }] } }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click()
  await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click(); await page.getByLabel('Recognition provider').selectOption('cloud'); await page.getByLabel('Deepgram API key').fill('test-key'); await page.getByRole('button', { name: 'Upload audio and generate' }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishTranscription)).toBe('function')
  await expect(page.getByRole('button', { name: 'Cancel transcription', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('transcription-progress', { state: 'completed', projectId: 'ui-test', jobId: 'old-job', progress: 1, error: null, cues: [{ id: 'stale', start: 0, end: 1, text: 'Do not replace' }] }))
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  await page.evaluate(() => { const state = (window as any).__test; state.holdTranscription = false; state.finishTranscription() })
  await page.getByRole('button', { name: 'Cancel transcription', exact: true }).click()
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', '')
  await page.getByLabel('Subtitle cue').selectOption('original'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Keep this cue')
  const jobs = await page.evaluate(() => { const calls = (window as any).__test.calls; return [calls.find((call: any) => call.name === 'start_transcription').args.jobId, calls.find((call: any) => call.name === 'cancel_transcription').args.jobId] })
  expect(jobs[0]).toBe(jobs[1]); expect(jobs[0]).not.toBe('old-job')
})

async function openOverlappingMusic(page: Page) {
  await mockedDesktop(page)
  await instrumentAudio(page)
  await page.goto('/')
  await page.evaluate(() => {
    const state = (window as any).__test
    const clip = { path: '/test-media/mix-audio.wav', sourceDuration: 12, start: 0, sourceStart: 0, duration: 9, volume: .15, fadeIn: 0, fadeOut: 0 }
    state.patchProject({ cameraPath: null, edits: { music: [
      { ...clip, id: 'first', name: 'First' }, { ...clip, id: 'second', name: 'Second', sourceStart: 1 },
    ] } })
  })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('tab', { name: 'Music', exact: true }).click()
  await page.getByLabel('Music clip', { exact: true }).selectOption('second')
  await expect.poll(() => page.evaluate(() => (window as any).__audioSources.length)).toBe(2)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
}

for (const edit of ['mute', 'delete', 'move']) test(`a pending music seek releases its decoder after ${edit} while keeping overlapping audio aligned`, async ({ page }) => {
  await openOverlappingMusic(page)
  await page.locator('audio[data-track="music: Second"]').evaluate(element => {
    Object.defineProperty(element, 'seeking', { configurable: true, get: () => true })
  })
  const seeking = page.getByText('Seeking edited video...', { exact: true })
  await page.getByLabel('Playhead position in seconds').evaluate(element => {
    (element as HTMLInputElement).value = '2'; element.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await expect(page.locator('audio[data-track="music: Second"]')).toHaveJSProperty('currentTime', 3)
  await expect(seeking).toBeVisible()
  if (edit === 'mute') await page.locator('#music-volume').press('Home')
  if (edit === 'delete') await page.getByRole('button', { name: 'Delete music clip' }).click()
  if (edit === 'move') { await page.getByLabel('Timeline start (s)').fill('7'); await page.getByLabel('Timeline start (s)').blur() }
  await expect(seeking).toBeHidden({ timeout: 2000 })
  await expect(page.locator('video[data-track="screen recording"]')).toHaveJSProperty('currentTime', 2)
  await expect(page.locator('audio[data-track="music: First"]')).toHaveJSProperty('currentTime', 2)
  await expect(page.locator('audio[data-track="music: First"]')).toHaveJSProperty('paused', true)
  await expect(page.getByRole('alert')).toBeHidden()
})

test('muting overlapping music leaves its decoder idle and the audible clip running', async ({ page }) => {
  await openOverlappingMusic(page)
  const screen = page.locator('video[data-track="screen recording"]')
  const first = page.locator('audio[data-track="music: First"]')
  const second = page.locator('audio[data-track="music: Second"]')
  const volume = page.locator('#music-volume')
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect(first).toHaveJSProperty('paused', false)
  await expect(second).toHaveJSProperty('paused', false)
  await volume.press('Home')
  await expect(second).toHaveJSProperty('paused', true)
  const idleTime = await second.evaluate(element => element.currentTime)
  const resets = await page.evaluate(() => (window as any).__limiterResets)
  const start = await screen.evaluate(element => element.currentTime)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(start + .6)
  expect(await second.evaluate(element => element.currentTime)).toBe(idleTime)
  expect(await page.evaluate(() => (window as any).__limiterResets)).toBe(resets)
  await expect(first).toHaveJSProperty('paused', false)
  const selection = page.getByLabel('Music clip', { exact: true })
  await selection.selectOption('first')
  await volume.press('Home')
  await expect(first).toHaveJSProperty('paused', true)
  const silentStart = await screen.evaluate(element => element.currentTime)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(silentStart + .3)
  expect(await second.evaluate(element => element.currentTime)).toBe(idleTime)
  await volume.press('ArrowRight')
  await expect(first).toHaveJSProperty('paused', false)
  await selection.selectOption('second')
  await volume.press('ArrowRight')
  await expect(second).toHaveJSProperty('paused', false)
  await expect.poll(() => second.evaluate(element => Math.abs(element.currentTime - 1 - document.querySelector<HTMLVideoElement>('video')!.currentTime))).toBeLessThan(.12)
  await page.getByRole('button', { name: 'Delete music clip' }).click()
  await expect(second).toHaveCount(0)
  await expect(first).toHaveJSProperty('paused', false)
  await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await expect(second).toHaveJSProperty('paused', false)
  await expect.poll(() => second.evaluate(element => Math.abs(element.currentTime - 1 - document.querySelector<HTMLVideoElement>('video')!.currentTime))).toBeLessThan(.12)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  await expect(page.getByRole('alert')).toBeHidden()
})

test('a future music clip enters at its in point without pausing the screen', async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { music: [
    { id: 'later', name: 'Later', path: '/test-media/mix-audio.wav', sourceDuration: 12, start: 1.2, sourceStart: 3, duration: 2, volume: .2, fadeIn: 0, fadeOut: 0 },
  ] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const screen = page.locator('video[data-track="screen recording"]')
  const clip = page.locator('audio[data-track="music: Later"]')
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(.1)
  await expect(clip).toHaveJSProperty('paused', true)
  await screen.evaluate(element => {
    element.dataset.pauses = '0'
    element.addEventListener('pause', () => { element.dataset.pauses = String(Number(element.dataset.pauses) + 1) })
  })
  await expect(clip).toHaveJSProperty('paused', false)
  await expect.poll(() => clip.evaluate(element => Math.abs(element.currentTime - 1.8 - document.querySelector<HTMLVideoElement>('video')!.currentTime))).toBeLessThan(.12)
  expect(await screen.getAttribute('data-pauses')).toBe('0')
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
})

test('exhausted and unreadable music files leave the remaining clip playable after scrubbing', async ({ page }) => {
  await mockedDesktop(page)
  await page.route('**/test-media/missing.wav', route => route.fulfill({ status: 404, body: '' }))
  await page.goto('/')
  await page.evaluate(() => {
    const state = (window as any).__test
    const clip = { sourceDuration: 12, start: 0, sourceStart: 0, duration: 6, volume: .2, fadeIn: 0, fadeOut: 0 }
    state.patchProject({ cameraPath: null, edits: { music: [
      { ...clip, id: 'short', name: 'Short', path: '/test-media/cut-zones.wav', sourceStart: 5.5 },
      { ...clip, id: 'missing', name: 'Missing', path: '/test-media/missing.wav' },
      { ...clip, id: 'good', name: 'Good', path: '/test-media/mix-audio.wav' },
    ] } })
  })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.locator('audio[data-track="music: Missing"]').evaluate(element => !!element.error)).toBe(true)
  await expect(page.getByRole('alert')).toContainText('Missing')
  await page.getByLabel('Playhead position in seconds').evaluate(element => {
    (element as HTMLInputElement).value = '2'; element.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden({ timeout: 2000 })
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  const screen = page.locator('video[data-track="screen recording"]')
  const good = page.locator('audio[data-track="music: Good"]')
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(2.7)
  await expect(good).toHaveJSProperty('paused', false)
  await expect.poll(() => good.evaluate(element => Math.abs(element.currentTime - document.querySelector<HTMLVideoElement>('video')!.currentTime))).toBeLessThan(.12)
  await expect(page.locator('audio[data-track="music: Short"]')).toHaveJSProperty('paused', true)
  await expect(page.locator('audio[data-track="music: Missing"]')).toHaveJSProperty('paused', true)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
})

test('muting a music clip whose natural start is delayed retires its late playback', async ({ page }) => {
  await mockedDesktop(page)
  await page.addInitScript(() => {
    const host = window as any
    const play = HTMLMediaElement.prototype.play
    HTMLMediaElement.prototype.play = function () {
      if (this.dataset.track !== 'music: Later') return play.call(this)
      return new Promise<void>((resolve, reject) => {
        host.__finishMusicStart = () => play.call(this).then(resolve, reject)
      })
    }
  })
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: null, edits: { music: [
    { id: 'later', name: 'Later', path: '/test-media/mix-audio.wav', sourceDuration: 12, start: 1, sourceStart: 1, duration: 3, volume: .2, fadeIn: 0, fadeOut: 0 },
  ] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('tab', { name: 'Music', exact: true }).click()
  await page.getByLabel('Music clip', { exact: true }).selectOption('later')
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__finishMusicStart)).toBe('function')
  await page.locator('#music-volume').press('Home')
  await page.evaluate(() => (window as any).__finishMusicStart())
  const screen = page.locator('video[data-track="screen recording"]')
  const start = await screen.evaluate(element => element.currentTime)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(start + .4)
  await expect(page.locator('audio[data-track="music: Later"]')).toHaveJSProperty('paused', true)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await expect(page.getByRole('alert')).toBeHidden()
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
})

test('imported music plays only within its clip and uses one limiter route after seek and undo', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Music', exact: true }).click(); await page.getByRole('button', { name: 'Import music' }).click()
  await page.getByLabel('Timeline start (s)').fill('2'); await page.getByLabel('Timeline start (s)').blur()
  await page.getByLabel('Audio in point (s)').fill('1'); await page.getByLabel('Audio in point (s)').blur()
  await page.getByLabel('Duration (s)', { exact: true }).fill('3'); await page.getByLabel('Duration (s)', { exact: true }).blur()
  await expect.poll(() => page.evaluate(() => (window as any).__audioSources.length)).toBe(1)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  expect(await page.locator('audio[data-track^="music:"]').evaluate(element => (element as HTMLAudioElement).paused)).toBe(true)
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '3'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await expect.poll(() => page.locator('audio[data-track^="music:"]').evaluate(element => (element as HTMLAudioElement).paused)).toBe(false)
  await expect.poll(() => page.locator('audio[data-track^="music:"]').evaluate(element => (element as HTMLAudioElement).currentTime)).toBeGreaterThan(2)
  expect(await page.evaluate(() => (window as any).__audioSources.length)).toBe(1)
  expect(await page.locator('audio[data-track^="music:"]').evaluate(element => (element as HTMLAudioElement).volume)).toBe(1)
  expect(await page.evaluate(() => { const w = window as any; return { gain: w.__audioGains.map((g: GainNode) => g.gain.value), context: w.__audioContexts[0].state, muted: document.querySelector('audio')!.muted } })).toEqual({ gain: [1, 1, .5], context: 'running', muted: false })
  await page.evaluate(() => { const w = window as any; w.__analyser = w.__audioContexts[0].createAnalyser(); w.__analyser.fftSize = 2048; w.__audioNodes[0].connect(w.__analyser) })
  const peak = () => page.evaluate(() => { const a = (window as any).__analyser; const data = new Float32Array(a.fftSize); a.getFloatTimeDomainData(data); return Math.max(...data.map(Math.abs)) })
  await expect.poll(peak).toBeGreaterThan(.3)
  expect(await peak()).toBeLessThan(.36)
  const resets = await page.evaluate(() => (window as any).__limiterResets)
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '6'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await expect.poll(() => page.evaluate(() => (window as any).__limiterResets)).toBeGreaterThan(resets)
  await expect.poll(() => page.locator('audio[data-track^="music:"]').evaluate(element => (element as HTMLAudioElement).paused)).toBe(true)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click(); await page.getByRole('button', { name: 'Delete music clip', exact: true }).click()
  await expect(page.locator('audio[data-track^="music:"]')).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect(page.locator('audio[data-track^="music:"]')).toHaveCount(1)
  await expect.poll(() => page.evaluate(() => (window as any).__audioSources.length)).toBe(2)
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('editor saves real control changes, zoom timing and export cancellation', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  await page.getByRole('button', { name: 'Play preview' }).click()
  await expect(page.getByRole('button', { name: 'Pause preview' })).toBeVisible()
  await page.waitForTimeout(400)
  await page.getByRole('button', { name: 'Pause preview' }).click()
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await page.getByLabel('Crop recording').check()
  await page.getByLabel('Left %').fill('10'); await page.getByLabel('Left %').blur()
  await page.getByRole('tab', { name: 'Camera' }).click()
  await page.getByLabel('Shape', { exact: true }).selectOption('rectangle')
  await page.getByLabel('Mirror webcam').uncheck()
  await page.getByRole('tab', { name: 'Cursor' }).click()
  await page.getByLabel('Show cursor').uncheck()
  await page.getByRole('button', { name: 'Add zoom', exact: true }).click()
  await expect(page.getByRole('tabpanel')).toContainText('Zoom segments')
  await page.getByRole('button', { name: 'Delete zoom' }).click()
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  const saved = await page.evaluate(() => (window as any).__test.saved)
  expect(saved.edits.aspectRatio).toBe('9:16'); expect(saved.edits.crop.x).toBe(.1)
  expect(saved.edits.camera.shape).toBe('rectangle'); expect(saved.edits.camera.mirror).toBe(false)
  expect(saved.edits.showCursor).toBe(false)
  await page.screenshot({ path: path.join(screenshotDir, 'editor-dark.png') })
  await page.getByRole('button', { name: /Switch to light/ }).click()
  await page.screenshot({ path: path.join(screenshotDir, 'editor-light.png') })
  await page.getByRole('button', { name: 'Export MP4' }).click()
  await page.getByRole('button', { name: 'Browse', exact: true }).click()
  await page.getByRole('button', { name: 'Export video', exact: true }).click()
  await expect(page.getByRole('progressbar')).toHaveAttribute('value', '0.4')
  await page.getByRole('button', { name: 'Cancel export', exact: true }).last().click()
  await expect(page.getByText('Export cancelled.', { exact: true })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toBeHidden()
  expect(errors).toEqual([])
})

test('recording chooses devices and keeps stop available after a device error', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'New recording' }).first().click()
  await page.getByLabel('Microphone', { exact: true }).selectOption('mic-test')
  await page.getByLabel('Webcam', { exact: true }).selectOption('cam-test')
  await expect(page.getByRole('dialog')).toContainText('Ctrl+Shift+R')
  await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.evaluate(() => (window as any).__test.emit('recording-error', 'Webcam disconnected'))
  await expect(page.getByRole('alert')).toContainText('Webcam disconnected')
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await expect(page.getByLabel('Project name')).toHaveValue('UI verification recording')
  const call = await page.evaluate(() => (window as any).__test.calls.find((c: any) => c.name === 'start_recording'))
  expect(call.args.options.cameraId).toBe('cam-test'); expect(call.args.options.microphoneId).toBe('mic-test')
})

test('narrow layout and keyboard tabs remain inside the viewport', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 900 }); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('tab', { name: 'Canvas' }).focus(); await page.keyboard.press('ArrowRight')
  await expect(page.getByRole('tab', { name: 'Zoom' })).toBeFocused()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await page.screenshot({ path: path.join(screenshotDir, 'editor-narrow.png'), fullPage: true })
})

test('closing flushes pending edits and cannot destroy an active recording', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByLabel('Project name').fill('Saved before close')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(true)
  const calls = await page.evaluate(() => (window as any).__test.calls)
  const saveIndex = calls.findIndex((c: any) => c.name === 'save_project_edits')
  const closeIndex = calls.findIndex((c: any) => c.name === 'close_app')
  expect(saveIndex).toBeGreaterThan(-1); expect(closeIndex).toBeGreaterThan(saveIndex)
  await page.getByRole('button', { name: 'New recording' }).first().click()
  await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.evaluate(() => { (window as any).__test.calls = []; (window as any).__test.emit('tauri://close-requested', null) })
  await expect(page.getByRole('alert')).toContainText('Stop recording or cancel export before closing.')
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app' || c.name === 'plugin:window|destroy'))).toBe(false)
})

test('a clean saved project closes through the backend without saving or destroying the window', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  await expect(page.getByText('Stored on this computer', { exact: true })).toHaveCount(0)
  await page.evaluate(() => {
    const test = (window as any).__test
    test.calls = []
    test.emit('tauri://close-requested', null)
    test.emit('tauri://close-requested', null)
  })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((c: any) => c.name === 'close_app').length)).toBe(1)
  const calls = await page.evaluate(() => (window as any).__test.calls.map((c: any) => c.name))
  expect(calls).not.toContain('save_project_edits')
  expect(calls).not.toContain('plugin:window|destroy')
})

test('closing waits for the pending save and shows its progress', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Project name').fill('Waiting for save')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect(page.getByText('Saving changes before closing...', { exact: true })).toBeVisible()
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  expect(await page.getByLabel('Aspect ratio').evaluate(element => { element.focus(); return document.activeElement === element })).toBe(false)
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(false)
  await page.evaluate(() => (window as any).__test.finishSave())
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(true)
  expect(await page.evaluate(() => (window as any).__test.saved.name)).toBe('Waiting for save')
})

async function openLongRecording(page: Page, clips: Array<{ id: string; start: number; end: number }>, camera = false) {
  const folder = path.resolve('artifacts/long-recording-ui')
  mkdirSync(folder, { recursive: true })
  const videoFile = path.join(folder, 'three-hours.mp4')
  const audioFile = path.join(folder, 'three-hours.m4a')
  const bundled = path.resolve('src-tauri/resources/ffmpeg.exe')
  const engine = process.env.SCREENRECORD_FFMPEG ?? (existsSync(bundled) ? bundled : 'ffmpeg')
  const encoder = process.platform === 'win32' ? ['-c:v', 'h264_mf', '-hw_encoding', '0'] : ['-c:v', 'libx264', '-preset', 'ultrafast']
  if (!existsSync(videoFile)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=green:s=160x96:r=1:d=10802', '-vf', "drawbox=c=red:t=fill:enable='gte(t,3601)*lt(t,10800)',drawbox=c=blue:t=fill:enable='gte(t,10800)'", ...encoder, '-b:v', '40000', '-g', '15', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', videoFile], { windowsHide: true })
  if (!existsSync(audioFile)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=8000:duration=10802', '-c:a', 'aac', '-b:a', '8k', '-movflags', '+faststart', audioFile], { windowsHide: true })
  await mockedDesktop(page)
  await instrumentAudio(page)
  await page.route('**/test-media/three-hours.mp4', route => serveMedia(route, videoFile, 'video/mp4', '*'))
  await page.route('**/test-media/three-hours.m4a', route => serveMedia(route, audioFile, 'audio/mp4', '*'))
  await page.goto('/')
  await page.evaluate(({ clips, camera }) => (window as any).__test.patchProject({
    screenPath: '/test-media/three-hours.mp4', systemAudioPath: '/test-media/three-hours.m4a', cameraPath: camera ? '/test-media/three-hours.mp4' : null,
    duration: 10802, width: 160, height: 96, fps: 30, cursor: [],
    edits: { trimEnd: 10802, zooms: [], showCursor: false, videoClips: clips },
  }), { clips, camera })
  await page.getByRole('button', { name: 'UI verification recording' }).click()
}

test('a three-hour source seeks, plays audio across a long cut, and reopens edits at the original timestamps', async ({ page }) => {
  test.setTimeout(120000)
  const startedAt = Date.now()
  await openLongRecording(page, [{ id: 'first-hour', start: 0, end: 3601 }, { id: 'last-seconds', start: 10800, end: 10802 }])
  const open = page.getByRole('button', { name: 'UI verification recording' })
  const screen = page.locator('video[data-track="screen recording"]')
  const audio = page.locator('audio[data-track="system audio"]')
  const playhead = page.getByLabel('Playhead position in seconds')
  await expect.poll(() => screen.evaluate(element => element.duration)).toBe(10802)
  await expect.poll(() => audio.evaluate(element => element.duration)).toBeCloseTo(10802, 1)
  await expect(playhead).toHaveAttribute('max', '3603')
  await seekEditor(page, 3600.1)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeCloseTo(3600.1, 4)
  await expect.poll(() => audio.evaluate(element => element.currentTime)).toBeCloseTo(3600.1, 4)
  await expect.poll(async () => (await screenPixel(page, .5, .5))[1]).toBeGreaterThan(100)
  await expect(playhead).toHaveAttribute('aria-valuetext', '1:00:00.1')
  await page.evaluate(async tapSource => {
    const host = window as any
    const source = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!
    const context = host.__audioContexts[0] as AudioContext
    const url = URL.createObjectURL(new Blob([tapSource], { type: 'text/javascript' }))
    try { await context.audioWorklet.addModule(url) } finally { URL.revokeObjectURL(url) }
    const tap = new AudioWorkletNode(context, 'pcm-tap')
    host.__longEvidence = { latePcmPeak: 0, removedRedDraws: 0, lateBlueDraws: 0 }
    tap.port.onmessage = event => {
      if (source.currentTime >= 10800 && !source.paused) host.__longEvidence.latePcmPeak = Math.max(host.__longEvidence.latePcmPeak, event.data.peak)
    }
    host.__audioGains[1].connect(tap); tap.connect(context.destination)
    const draw = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) {
      const result = (draw as any).apply(this, args)
      if (this.canvas.classList.contains('preview-canvas')) {
        const [r, g, b] = this.getImageData(this.canvas.width / 2, this.canvas.height / 2, 1, 1).data
        if (r > 160 && g < 60 && b < 60) host.__longEvidence.removedRedDraws++
        if (source.currentTime >= 10800 && b > 220 && r < 60 && g < 60) host.__longEvidence.lateBlueDraws++
      }
      return result
    } as typeof draw
  }, pcmTapSource)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(10800)
  await expect.poll(async () => (await screenPixel(page, .5, .5))[2]).toBeGreaterThan(220)
  await expect.poll(() => audio.evaluate(element => !element.paused && Math.abs(element.currentTime - document.querySelector<HTMLVideoElement>('video')!.currentTime) < .12)).toBe(true)
  await expect.poll(() => page.evaluate(() => (window as any).__longEvidence.latePcmPeak)).toBeGreaterThan(.02)
  const evidence = await page.evaluate(() => (window as any).__longEvidence)
  expect(evidence.removedRedDraws).toBe(0)
  expect(evidence.lateBlueDraws).toBeGreaterThan(0)
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  await seekEditor(page, 3602)
  await page.getByRole('button', { name: 'Split', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.videoClips.map((clip: any) => [clip.start, clip.end]))).toEqual([[0, 3601], [10800, 10801], [10801, 10802]])
  await open.click()
  await expect(page.getByText('3 clips / 1:00:03.0')).toBeVisible()
  await seekEditor(page, 3602)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeCloseTo(10801, 4)
  await expect.poll(() => audio.evaluate(element => element.currentTime)).toBeCloseTo(10801, 4)
  await expect(page.getByRole('alert')).toBeHidden()
  writeEvidence('artifacts/long-recording-ui/long-cut-evidence.json', { durationSeconds: 10802, removedSeconds: 7199, elapsedSeconds: (Date.now() - startedAt) / 1000, ...evidence })
})

if (process.env.SCREENRECORD_UI_SOAK === '1') test('ten-minute editor playback soak across hour-scale cuts and live track changes', async ({ page }) => {
  test.setTimeout(720000)
  const output = path.resolve('artifacts/reliability-0.7.4')
  mkdirSync(output, { recursive: true })
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await openLongRecording(page, [{ id: 'first-hour', start: 0, end: 3601 }, { id: 'late', start: 10740, end: 10802 }], true)
  await page.getByRole('button', { name: 'Audition', exact: true }).click()
  const a = page.getByLabel('Loop start (seconds)')
  const b = page.getByLabel('Loop end (seconds)')
  await b.fill('3660'); await b.blur()
  await a.fill('3595'); await a.blur()
  await page.getByLabel('Loop selection', { exact: true }).check()
  await seekEditor(page, 3595)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => element.currentTime)).toBeGreaterThan(3595.1)
  await page.evaluate(() => {
    const screen = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!
    const camera = document.querySelector<HTMLVideoElement>('video[data-track="webcam recording"]')!
    const audio = document.querySelector<HTMLAudioElement>('audio[data-track="system audio"]')!
    let last = screen.currentTime
    let lastAdvance = performance.now()
    const start = lastAdvance
    const metrics = { samples: 0, advancingSeconds: 0, maxStationaryMs: 0, forwardJumps: 0, backwardJumps: 0, maxCameraDrift: 0, maxAudioDrift: 0, cameraSamples: 0, audioSamples: 0, elapsed: 0, peakHeapBytes: 0 }
    const host = window as any
    host.readSoak = () => ({ ...metrics, heapBytes: (performance as any).memory?.usedJSHeapSize, time: screen.currentTime, audioContexts: host.__audioContexts.length, mediaElements: document.querySelectorAll('audio,video').length })
    const timer = setInterval(() => {
      const now = performance.now()
      const time = screen.currentTime
      const delta = time - last
      metrics.samples++; metrics.elapsed = (now - start) / 1000
      metrics.maxStationaryMs = Math.max(metrics.maxStationaryMs, now - lastAdvance)
      if (Math.abs(delta) > .001) {
        if (delta > 1.5) metrics.forwardJumps++
        else if (delta < 0) metrics.backwardJumps++
        else metrics.advancingSeconds += delta
        lastAdvance = now
      }
      if (!screen.paused && !screen.seeking && screen.playbackRate > 0) {
        if (!camera.paused && !camera.seeking && camera.playbackRate > 0) { metrics.cameraSamples++; metrics.maxCameraDrift = Math.max(metrics.maxCameraDrift, Math.abs(camera.currentTime - time)) }
        if (!audio.paused && !audio.seeking && audio.playbackRate > 0) { metrics.audioSamples++; metrics.maxAudioDrift = Math.max(metrics.maxAudioDrift, Math.abs(audio.currentTime - time)) }
      }
      metrics.peakHeapBytes = Math.max(metrics.peakHeapBytes, (performance as any).memory?.usedJSHeapSize ?? 0)
      last = time
    }, 200)
    host.stopSoak = () => clearInterval(timer)
  })
  const adjustments: string[] = []
  const checkpoints: Array<Record<string, number>> = []
  const start = Date.now()
  const volume = page.getByRole('slider', { name: 'Computer audio volume' })
  try {
    for (let step = 0; step < 20; step++) {
      await page.waitForTimeout(Math.max(0, start + (step + 1) * 30000 - Date.now()))
      const point = await page.evaluate(() => (window as any).readSoak())
      checkpoints.push(point)
      writeEvidence(path.join(output, 'editor-10min-soak.json'), { durationSeconds: 10802, elapsedSeconds: (Date.now() - start) / 1000, adjustments, checkpoints, errors })
      expect(point.maxStationaryMs).toBeLessThan(5000)
      await expect(page.getByRole('button', { name: 'Pause preview', exact: true })).toBeVisible()
      await expect(page.getByRole('alert')).toBeHidden()
      if (step === 19) break
      switch (step % 6) {
        case 0:
          await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await volume.press('Home')
          await expect(page.locator('audio[data-track="system audio"]')).toHaveJSProperty('paused', true)
          adjustments.push('mute computer audio'); break
        case 1:
          await volume.press('End'); await expect(page.locator('audio[data-track="system audio"]')).toHaveJSProperty('paused', false)
          adjustments.push('restore computer audio'); break
        case 2:
          await page.getByRole('tab', { name: 'Camera', exact: true }).click(); await page.locator('#camera-x').press('ArrowLeft'); await page.getByLabel('Show webcam').uncheck()
          await expect(page.locator('video[data-track="webcam recording"]')).toHaveJSProperty('paused', true)
          adjustments.push('move and hide webcam'); break
        case 3:
          await page.getByLabel('Show webcam').check(); await expect(page.locator('video[data-track="webcam recording"]')).toHaveJSProperty('paused', false)
          adjustments.push('restore webcam'); break
        case 4: await seekEditor(page, 3658); adjustments.push('seek near loop end'); break
        case 5: await seekEditor(page, 3600); adjustments.push('seek before cut'); break
      }
    }
  } finally {
    await page.evaluate(() => (window as any).stopSoak())
    await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  }
  const last = checkpoints.at(-1)!
  expect(last.elapsed).toBeGreaterThan(599)
  expect(last.advancingSeconds).toBeGreaterThan(560)
  expect(last.forwardJumps).toBeGreaterThan(5)
  expect(last.backwardJumps).toBeGreaterThan(5)
  expect(last.audioSamples).toBeGreaterThan(1000)
  expect(last.cameraSamples).toBeGreaterThan(1000)
  expect(last.maxCameraDrift).toBeLessThan(.25)
  expect(last.maxAudioDrift).toBeLessThan(.25)
  expect(last.audioContexts).toBe(1)
  expect(errors).toEqual([])
})

test('a save failure preserves edits and permits retry without destroying the window', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.saveError = 'Project folder is not writable.' })
  await page.getByLabel('Project name').fill('Keep these edits')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect(page.getByRole('alert')).toContainText('Project folder is not writable.')
  await expect(page.getByLabel('Project name')).toHaveValue('Keep these edits')
  await expect(page.getByLabel('Project name')).toBeEnabled()
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app' || c.name === 'plugin:window|destroy'))).toBe(false)
  await page.evaluate(() => { const test = (window as any).__test; test.saveError = null; test.emit('tauri://close-requested', null) })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(true)
})

test('a disk-full autosave overlapping close keeps later edits and clears its error after a successful retry', async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await page.getByLabel('Screen sizing').selectOption('fill')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect(page.getByText('Saving changes before closing...')).toBeVisible()
  await page.evaluate(() => {
    const state = (window as any).__test
    state.holdSave = false
    state.saveError = 'Could not save project.json: There is not enough space on the disk (os error 112)'
    state.finishSave()
  })
  await expect(page.getByRole('alert')).toContainText('os error 112')
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible()
  await expect(page.getByLabel('Screen sizing')).toHaveValue('fill')
  expect(await page.evaluate(() => (window as any).__test.saved)).toBeNull()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(false)
  await page.evaluate(() => { (window as any).__test.saveError = null })
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.saved.edits)).toMatchObject({ aspectRatio: '1:1', fitMode: 'fill' })
  await expect(page.getByRole('alert')).toBeHidden()
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'close_app').length)).toBe(1)
})

test('backend close errors stay visible and never fall back to destructive closing', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { const test = (window as any).__test; test.closeError = 'Command close_app not found'; test.emit('tauri://close-requested', null) })
  await expect(page.getByRole('alert')).toContainText('Command close_app not found')
  await expect(page.getByLabel('Project name')).toBeEnabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'plugin:window|destroy'))).toBe(false)
})

test('a stalled save reports a bounded close error and does not close later without a new request', async ({ page }) => {
  await mockedDesktop(page); await page.clock.install(); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Project name').fill('Save is pending')
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect(page.getByText('Saving changes before closing...', { exact: true })).toBeVisible()
  await page.clock.fastForward(10_100)
  await expect(page.getByRole('alert')).toContainText('Closing is taking too long.')
  await expect(page.getByLabel('Project name')).toBeEnabled()
  await page.evaluate(() => (window as any).__test.finishSave())
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app' || c.name === 'plugin:window|destroy'))).toBe(false)
  await page.evaluate(() => (window as any).__test.emit('tauri://close-requested', null))
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'close_app'))).toBe(true)
})

test('preparing and stopping a recording preserves the previous project edits', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => {
    const test = (window as any).__test
    test.holdSave = true
    test.recordedProject = { ...test.currentProject(), id: 'new-recording', name: 'New recording result' }
  })
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await page.getByRole('button', { name: 'New recording' }).first().click()
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  expect(await page.getByLabel('Aspect ratio').evaluate(element => { element.focus(); return document.activeElement === element })).toBe(false)
  await expect(page.getByRole('dialog')).toBeHidden()
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'start_recording'))).toBe(false)
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = false; test.finishSave() })
  await page.getByRole('button', { name: 'Start recording', exact: true }).click()
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  expect(await page.getByLabel('Aspect ratio').evaluate(element => { element.focus(); return document.activeElement === element })).toBe(false)
  await expect(page.getByRole('button', { name: 'Stop recording', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click()
  await expect(page.getByLabel('Project name')).toHaveValue('New recording result')
  expect(await page.evaluate(() => (window as any).__test.saved.edits.aspectRatio)).toBe('9:16')
  expect(await page.evaluate(() => (window as any).__test.saved.id)).toBe('ui-test')
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', '')
})

test('auto zoom flushes pending edits and locks the inspector until its delayed result arrives', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = true; test.holdRegenerate = true })
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await page.getByRole('tab', { name: 'Zoom', exact: true }).click()
  await page.getByRole('button', { name: 'Auto zoom', exact: true }).click()
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'regenerate_zooms'))).toBe(false)
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = false; test.finishSave() })
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishRegenerate)).toBe('function')
  await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  await expect(page.getByText('Generating zooms...', { exact: true })).toBeVisible()
  expect(await page.locator('#tab-canvas').evaluate(element => { element.focus(); return document.activeElement === element })).toBe(false)
  await page.evaluate(() => (window as any).__test.finishRegenerate())
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', '')
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click()
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('9:16')
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name))
  expect(calls.indexOf('save_project_edits')).toBeLessThan(calls.indexOf('regenerate_zooms'))
  expect(calls.filter((name: string) => name === 'regenerate_zooms')).toHaveLength(1)
})

test('a save failure prevents auto zoom from replacing unsaved edits', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.saveError = 'The project disk is full.' })
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await page.getByRole('tab', { name: 'Zoom', exact: true }).click()
  await page.getByRole('button', { name: 'Auto zoom', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('The project disk is full.')
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', '')
  expect(await page.evaluate(() => (window as any).__test.calls.some((c: any) => c.name === 'regenerate_zooms'))).toBe(false)
  await page.getByRole('tab', { name: 'Canvas', exact: true }).click()
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('1:1')
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible()
})

test('a delayed stopped event for the open recording does not overwrite newer local edits', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = true; test.recordedProject = test.currentProject() })
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await page.evaluate(() => { const test = (window as any).__test; test.emit('recording-stopped', test.recordedProject) })
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('9:16')
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = false; test.finishSave() })
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.aspectRatio)).toBe('9:16')
})

test('autosave sends only mutable edits and preserves newer changes while a save is pending', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.evaluate(() => { (window as any).__test.holdSave = true })
  await page.getByLabel('Aspect ratio').selectOption('9:16')
  await page.getByRole('button', { name: 'Save', exact: true }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  await page.getByLabel('Aspect ratio').selectOption('1:1')
  await page.evaluate(() => { const test = (window as any).__test; test.holdSave = false; test.finishSave() })
  await expect(page.getByLabel('Aspect ratio')).toHaveValue('1:1')
  await expect(page.getByText('Saved', { exact: true })).toBeVisible()
  const saves = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'save_project_edits'))
  expect(saves.length).toBeGreaterThanOrEqual(2)
  expect(Object.keys(saves[0].args).sort()).toEqual(['edits', 'id', 'name'])
  expect(JSON.stringify(saves[0].args)).not.toContain('screenPath')
  expect(saves.at(-1).args.edits.aspectRatio).toBe('1:1')
  expect(await page.evaluate(() => (window as any).__test.currentProject().cursor.length)).toBe(121)
})

test('keyboard activation stays with buttons and inspector tabs without moving the playhead', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const position = await page.getByLabel('Playhead position in seconds').inputValue()
  await page.getByRole('tab', { name: 'Canvas', exact: true }).focus()
  await page.keyboard.press('ArrowRight')
  await expect(page.getByRole('tab', { name: 'Zoom', exact: true })).toBeFocused()
  await expect(page.getByLabel('Playhead position in seconds')).toHaveValue(position)
  await page.getByRole('button', { name: 'Add zoom', exact: true }).focus()
  await page.keyboard.press('Space')
  await expect(page.getByRole('button', { name: 'Delete zoom', exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeVisible()
  await page.evaluate(() => (document.activeElement as HTMLElement)?.blur())
  await page.keyboard.press('Space')
  await expect(page.getByRole('button', { name: 'Pause preview', exact: true })).toBeVisible()
})

test('webcam offset is applied and short webcam footage disappears at its actual EOF', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ cameraPath: '/test-media/camera-short.mp4', cameraOffsetSeconds: .25, edits: { padding: 180, camera: { enabled: true, x: 0, y: .2, width: .1, shape: 'rectangle', mirror: false } } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  const seek = async (time: number) => {
    await page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, time)
  }
  const pixel = () => page.locator('canvas').evaluate(element => [...element.getContext('2d')!.getImageData(Math.round(element.width * .05), Math.round(element.height * .2 + element.width * .05), 1, 1).data].slice(0, 3))
  await seek(.25)
  await expect.poll(() => page.evaluate(() => {
    const cam = document.querySelector<HTMLVideoElement>('video[data-track="webcam recording"]')!
    const screen = document.querySelector<HTMLVideoElement>('video[data-track="screen recording"]')!
    return { synchronized: Math.abs(cam.currentTime - screen.currentTime - .25) < .04, camera: cam.currentTime, screen: screen.currentTime, ready: cam.readyState, duration: cam.duration, source: cam.currentSrc, seekable: [...Array(cam.seekable.length)].map((_, index) => [cam.seekable.start(index), cam.seekable.end(index)]) }
  })).toMatchObject({ synchronized: true })
  await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
  await seek(.9)
  await expect.poll(pixel).toEqual([40, 53, 68])
  await seek(.25)
  await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
})

test('webcam clock preserves legacy EOF at 35.5 minus offset and uses new elapsed-time media without stretching', async ({ page }) => {
  await page.addInitScript(() => {
    const events: unknown[] = []; (window as any).__mediaClockEvents = events
    const property = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'currentTime')!
    Object.defineProperty(HTMLMediaElement.prototype, 'currentTime', { configurable: true, get: property.get, set(value: number) { events.push({ event: 'set', track: this.dataset.track, value, current: property.get!.call(this), stack: new Error().stack }); property.set!.call(this, value) } })
    for (const name of ['seeking', 'seeked', 'loadeddata', 'ended']) document.addEventListener(name, event => { const media = event.target as HTMLMediaElement; events.push({ event: name, track: media.dataset?.track, current: media.currentTime, ready: media.readyState, seeking: media.seeking }); if (events.length > 100) events.shift() }, true)
  })
  await mockedDesktop(page)
  const offset = .790257
  for (const media of ['legacy', 'realtime']) {
    await page.goto('/')
    await page.evaluate(({ media, offset }) => (window as any).__test.patchProject({ screenPath: '/test-media/screen-clock.mp4', cameraPath: `/test-media/camera-clock-${media}.mp4`, cameraOffsetSeconds: offset, cameraIntervals: null, width: 320, height: 180, duration: 36.5, edits: { trimStart: 0, trimEnd: 36.5, padding: 180, zooms: [], camera: { enabled: true, x: 0, y: .2, width: .1, shape: 'rectangle', mirror: false } } }), { media, offset })
    await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
    const camera = page.locator('video[data-track="webcam recording"]'); const screen = page.locator('video[data-track="screen recording"]')
    const pixel = () => page.locator('canvas').evaluate(element => [...element.getContext('2d')!.getImageData(Math.round(element.width * .05), Math.round(element.height * .2 + element.width * .05), 1, 1).data].slice(0, 3))
    await expect.poll(() => camera.evaluate(element => element.duration)).toBe(media === 'legacy' ? 35.5 : 36.5)
    if (media === 'legacy') expect(await camera.evaluate(element => element.duration) - offset).toBeCloseTo(34.709743, 6)
    await seekEditor(page, 34.7); await expect.poll(() => camera.evaluate(element => element.currentTime)).toBeCloseTo(35.490257, 5); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
    await seekEditor(page, 34.733333); await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(34.709743)
    if (media === 'legacy') await expect.poll(pixel).toEqual([40, 53, 68])
    else await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
    await seekEditor(page, 35.5)
    if (media === 'legacy') await expect.poll(pixel).toEqual([40, 53, 68])
    else { await expect.poll(() => camera.evaluate(element => element.currentTime)).toBeCloseTo(36.290257, 5); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220) }
    await seekEditor(page, 34.7); await expect.poll(async () => (await pixel())[2]).toBeGreaterThan(220)
  }
})

async function instrumentAudio(page: Page, failModule = false) {
  await page.addInitScript(fail => {
    const w = window as any
    w.__audioContexts = []; w.__audioNodes = []; w.__audioSources = []; w.__audioGains = []; w.__audioMessages = []; w.__limiterResets = 0
    const BaseContext = window.AudioContext
    w.AudioContext = class extends BaseContext {
      constructor(options: AudioContextOptions) {
        super(options); w.__audioContexts.push(this)
        if (fail) this.audioWorklet.addModule = async () => { throw new Error('Test worklet is unavailable') }
      }
      createMediaElementSource(element: HTMLMediaElement) { w.__audioSources.push(element); return super.createMediaElementSource(element) }
      createGain() { const gain = super.createGain(); w.__audioGains.push(gain); return gain }
    }
    const BaseNode = window.AudioWorkletNode
    w.AudioWorkletNode = class extends BaseNode {
      constructor(context: BaseAudioContext, name: string, options: AudioWorkletNodeOptions) { super(context, name, options); w.__audioNodes.push(this); const send = this.port.postMessage.bind(this.port); this.port.postMessage = (message: any) => { if (message === 'reset') w.__limiterResets++; w.__audioMessages.push({ name, message }); send(message) } }
    }
  }, failModule)
}

for (const outcome of ['camera timeout', 'screen timeout', 'hide camera']) test(`pending media startup recovers after ${outcome} and ignores late completion`, async ({ page }) => {
  await mockedDesktop(page)
  const required = outcome === 'screen timeout'
  const heldTrack = required ? 'screen recording' : 'webcam recording'
  await page.addInitScript(track => {
    const play = HTMLMediaElement.prototype.play
    HTMLMediaElement.prototype.play = function () {
      if (this.dataset.track !== track || this.dataset.held) return play.call(this)
      this.dataset.held = 'true'
      return new Promise<void>((resolve, reject) => {
        ;(window as any).finishHeldStart = () => play.call(this).then(resolve, reject)
      })
    }
  }, heldTrack)
  await page.goto('/')
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await page.clock.install()
  const play = page.getByRole('button', { name: 'Play preview', exact: true })
  await play.click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).finishHeldStart)).toBe('function')
  if (outcome === 'hide camera') {
    await page.getByRole('tab', { name: 'Camera', exact: true }).click()
    await page.getByLabel('Show webcam').uncheck()
    await expect(page.getByRole('alert')).toHaveCount(0)
  } else {
    await page.clock.fastForward(7100)
    await expect(page.getByRole('alert')).toContainText(`Starting ${heldTrack} took too long`)
  }
  const screen = page.locator('video[data-track="screen recording"]')
  if (required) {
    await expect(play).toBeEnabled()
    await expect(screen).toHaveJSProperty('paused', true)
    await play.click()
  }
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(.35)
  await page.evaluate(() => (window as any).finishHeldStart())
  if (required) {
    await expect(screen).toHaveJSProperty('paused', false)
  } else {
    await expect(page.locator('video[data-track="webcam recording"]')).toHaveJSProperty('paused', true)
  }
  const before = await screen.evaluate(element => element.currentTime)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(before + .3)
})

for (const delayedTrack of ['webcam recording', 'system audio']) test(`slow ${delayedTrack} startup keeps the other preview clocks in sync`, async ({ page }) => {
  await mockedDesktop(page)
  await page.addInitScript(track => {
    const play = HTMLMediaElement.prototype.play
    HTMLMediaElement.prototype.play = async function () {
      this.dataset.playAttempts = String(Number(this.dataset.playAttempts ?? 0) + 1)
      if (this.dataset.track === track) await new Promise(resolve => setTimeout(resolve, 220))
      return play.call(this)
    }
  }, delayedTrack)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    systemAudioPath: '/test-media/mix-audio.wav', cameraOffsetSeconds: .25,
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  const screen = page.locator('video[data-track="screen recording"]')
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(1.2)
  expect(await screen.getAttribute('data-play-attempts')).toBe('1')
  const drift = await screen.evaluate(element => {
    const camera = document.querySelector<HTMLVideoElement>('video[data-track="webcam recording"]')!
    const system = document.querySelector<HTMLAudioElement>('audio[data-track="system audio"]')!
    return Math.max(Math.abs(camera.currentTime - .25 - element.currentTime), Math.abs(system.currentTime - element.currentTime))
  })
  expect(drift).toBeLessThan(.12)
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('muting recorded audio and hiding or moving the webcam keeps screen playback advancing', async ({ page }) => {
  await mockedDesktop(page)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({
    systemAudioPath: '/test-media/mix-audio.wav', cameraOffsetSeconds: .25,
  }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const screen = page.locator('video[data-track="screen recording"]')
  const camera = page.locator('video[data-track="webcam recording"]')
  const system = page.locator('audio[data-track="system audio"]')
  const seeking = page.getByText('Seeking edited video...', { exact: true })
  const time = () => screen.evaluate(element => element.currentTime)
  async function expectContinuousPlayback() {
    const start = await time()
    await expect.poll(time).toBeGreaterThan(start + .5)
    expect(await screen.evaluate(element => Number(element.dataset.pauses))).toBe(0)
    await expect(seeking).toBeHidden()
  }
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(time).toBeGreaterThan(.2)
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  const volume = page.getByRole('slider', { name: 'Computer audio volume' })
  await volume.press('Home')
  await expect(system).toHaveJSProperty('paused', true)
  await screen.evaluate(element => {
    element.dataset.pauses = '0'
    element.addEventListener('pause', () => { element.dataset.pauses = String(Number(element.dataset.pauses) + 1) })
  })
  await page.getByRole('tab', { name: 'Camera', exact: true }).click()
  const position = page.locator('#camera-x')
  await position.press('Home')
  await position.press('ArrowRight')
  await expect(position).toHaveValue('0.01')
  await expectContinuousPlayback()
  const webcam = page.getByLabel('Show webcam')
  await webcam.uncheck()
  await expect(camera).toHaveJSProperty('paused', true)
  const inactiveCameraTime = await camera.evaluate(element => element.currentTime)
  await expectContinuousPlayback()
  expect(await camera.evaluate(element => element.currentTime)).toBe(inactiveCameraTime)

  await webcam.check()
  await expect(camera).toHaveJSProperty('paused', false)
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  await volume.press('End')
  await expect(system).toHaveJSProperty('paused', false)
  await expect.poll(() => screen.evaluate(element => {
    const camera = document.querySelector<HTMLVideoElement>('video[data-track="webcam recording"]')!
    const system = document.querySelector<HTMLAudioElement>('audio[data-track="system audio"]')!
    return Math.max(Math.abs(camera.currentTime - .25 - element.currentTime), Math.abs(system.currentTime - element.currentTime))
  })).toBeLessThan(.12)
  await screen.evaluate(element => { element.dataset.pauses = '0' })
  await volume.press('ArrowLeft')
  await expectContinuousPlayback()
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
})

test('audio preview mixes once through the limiter and resumes, seeks and disposes its context', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ microphonePath: '/test-media/mix-audio.wav', systemAudioPath: '/test-media/mix-audio.wav' }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioSources.length)).toBe(2)
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('running')
  await page.evaluate(() => {
    const w = window as any; w.__analyser = w.__audioContexts[0].createAnalyser(); w.__analyser.fftSize = 2048; w.__audioNodes[0].connect(w.__analyser)
  })
  const peak = () => page.evaluate(() => { const a = (window as any).__analyser; const samples = new Float32Array(a.fftSize); a.getFloatTimeDomainData(samples); return Math.max(...samples.map(Math.abs)) })
  await expect.poll(peak).toBeGreaterThan(.8)
  expect(await peak()).toBeLessThanOrEqual(.951)
  expect(await page.locator('audio').evaluateAll(elements => elements.map(element => (element as HTMLAudioElement).volume))).toEqual([1, 1])
  await page.getByRole('tab', { name: 'Audio', exact: true }).click()
  for (const selector of ['#mic-volume', '#system-volume']) await page.locator(selector).evaluate(element => { (element as HTMLInputElement).value = '.5'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await expect.poll(() => page.evaluate(() => (window as any).__audioGains.map((gain: GainNode) => gain.gain.value))).toEqual([1, 1, .5, .5])
  expect(await page.locator('audio').evaluateAll(elements => elements.map(element => (element as HTMLAudioElement).volume))).toEqual([1, 1])
  await page.getByRole('button', { name: 'Pause preview', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('suspended')
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '4'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('running')
  await expect.poll(() => page.locator('audio').first().evaluate(element => (element as HTMLAudioElement).currentTime)).toBeGreaterThan(4)
  expect(await page.evaluate(() => (window as any).__audioSources.length)).toBe(2)
  await page.evaluate(() => (window as any).__audioNodes[0].dispatchEvent(new Event('processorerror')))
  await expect(page.getByRole('alert')).toContainText('Audio preview processing failed')
  await expect.poll(() => page.locator('audio').evaluateAll(elements => elements.every(element => (element as HTMLAudioElement).paused))).toBe(true)
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('closed')
})

for (const blocked of ['worklet', 'resume']) test(`a blocked audio ${blocked} can be paused and retried while video recovers silently`, async ({ page }) => {
  await mockedDesktop(page)
  await instrumentAudio(page)
  await page.goto('/')
  await page.evaluate(blocked => {
    const state = (window as any).__test
    const held = new Promise<void>(resolve => { state.releaseAudioStartup = resolve })
    state.audioWaits = 0
    const Context = window.AudioContext
    window.AudioContext = class extends Context {
      constructor(options?: AudioContextOptions) {
        super(options)
        if (blocked === 'worklet') {
          const load = this.audioWorklet.addModule.bind(this.audioWorklet)
          this.audioWorklet.addModule = async (...args) => { state.audioWaits++; await held; return load(...args) }
        } else {
          const resume = this.resume.bind(this)
          this.resume = async () => { state.audioWaits++; await held; return resume() }
        }
      }
    }
    state.patchProject({ cameraPath: null, systemAudioPath: '/test-media/mix-audio.wav', edits: { music: [
      { id: 'later', name: 'Later', path: '/test-media/mix-audio.wav', sourceDuration: 12, start: .4, sourceStart: 1, duration: 7, volume: .2, fadeIn: 0, fadeOut: 0 },
    ] } })
  }, blocked)
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  const play = page.getByRole('button', { name: 'Play preview', exact: true })
  const pause = page.getByRole('button', { name: 'Pause preview', exact: true })
  const screen = page.locator('video[data-track="screen recording"]')
  const system = page.locator('audio[data-track="system audio"]')
  const music = page.locator('audio[data-track="music: Later"]')
  await play.click()
  await expect.poll(() => page.evaluate(() => (window as any).__test.audioWaits)).toBeGreaterThan(0)
  await pause.click()
  await expect(screen).toHaveJSProperty('paused', true)
  await play.click()
  await expect.poll(() => screen.evaluate(element => element.currentTime), { timeout: 10000 }).toBeGreaterThan(.6)
  await expect(page.getByRole('alert')).toContainText('audio', { ignoreCase: true })
  await expect(system).toHaveJSProperty('paused', true)
  await expect(music).toHaveJSProperty('paused', true)
  await seekEditor(page, 3)
  await expect.poll(() => screen.evaluate(element => element.currentTime)).toBeGreaterThan(3.1)
  await page.evaluate(() => (window as any).__test.releaseAudioStartup())
  await expect.poll(() => page.evaluate(() => (window as any).__audioSources.length)).toBe(2)
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('suspended')
  expect(await page.evaluate(() => (window as any).__audioGains.slice(0, 2).map((gate: GainNode) => gate.gain.value))).toEqual([0, 0])
  await expect(system).toHaveJSProperty('paused', true)
  await expect(music).toHaveJSProperty('paused', true)
  await pause.click()
  await play.click()
  await expect(system).toHaveJSProperty('paused', false)
  await expect(music).toHaveJSProperty('paused', false)
  await expect.poll(() => page.evaluate(() => (window as any).__audioContexts[0].state)).toBe('running')
  await pause.click()
})

test('audio processing failure stays visible while screen preview continues without direct audio fallback', async ({ page }) => {
  await mockedDesktop(page); await instrumentAudio(page, true)
  await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ systemAudioPath: '/test-media/audio.wav' }))
  await page.getByRole('button', { name: 'UI verification recording' }).click()
  await expect(page.getByRole('alert')).toContainText('Audio preview could not start')
  await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Pause preview', exact: true })).toBeVisible()
  expect(await page.locator('audio').evaluate(element => (element as HTMLAudioElement).paused && (element as HTMLAudioElement).muted)).toBe(true)
})



test('SRT import flushes edits, preserves cross-cut source ranges, undoes once and keeps cues after failure', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const t = (window as any).__test; t.patchProject({ edits: { videoClips: [{ id: 'a', start: 0, end: 3 }, { id: 'b', start: 6, end: 9 }], subtitles: [{ id: 'old', start: 0, end: 1, text: 'Original caption' }] } }); t.importedCues = [{ id: 'cross', start: 2, end: 7, text: 'Imported across a cut', words: [], sourceRanges: [{ start: 2, end: 3 }, { start: 6, end: 7 }] }] })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByLabel('Project name').fill('Import target'); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click()
  await page.getByRole('button', { name: 'Import SRT', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Imported across a cut')
  await expect(page.getByText('This cue spans separate source ranges. It stays hidden between those ranges.')).toBeVisible()
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name)); expect(calls.indexOf('save_project_edits')).toBeLessThan(calls.indexOf('import_subtitles'))
  await page.getByLabel('Highlight spoken words').check(); await expect(page.getByText(/1 cue has no word timing/)).toBeVisible(); await page.getByRole('button', { name: 'Undo', exact: true }).click()
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('old'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Original caption')
  await page.evaluate(() => { (window as any).__test.srtError = 'SRT cue 2 has an invalid timestamp.' }); await page.getByRole('button', { name: 'Import SRT', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('invalid timestamp'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('Original caption')
})

test('timed subtitle split and merge retain words with one undo and text editing removes fake timing', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { subtitles: [{ id: 'timed', start: 0, end: 4, text: 'one two three four', words: ['one', 'two', 'three', 'four'].map((text, start) => ({ text, start, end: start + .8 })), sourceRanges: [] }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('timed')
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '1.8'; element.dispatchEvent(new Event('input', { bubbles: true })) }); await page.getByRole('button', { name: 'Split at playhead', exact: true }).click()
  await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('three four'); await page.getByRole('button', { name: 'Save', exact: true }).click()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.subtitles.map((cue: any) => cue.words.length))).toEqual([2, 2])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('timed'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('one two three four'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('timed'); await page.getByRole('button', { name: 'Merge next cue', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('one two three four')
  await page.getByLabel('Subtitle text', { exact: true }).fill('Edited words'); await page.getByLabel('Subtitle text', { exact: true }).blur(); await page.getByRole('button', { name: 'Save', exact: true }).click()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.subtitles[0].words)).toEqual([])
  await expect(page.getByText('Text changed. This cue now uses plain timing; generate again to restore word timing.')).toBeVisible()
})

test('merging projected captions never restores removed private words or fills source gaps', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { videoClips: [{ id: 'a', start: 0, end: 1 }, { id: 'b', start: 3, end: 9 }], subtitles: [{ id: 'first', start: 0, end: 3, text: 'public secret', words: [{ start: 0, end: .8, text: 'public' }, { start: 1, end: 2, text: 'secret' }] }, { id: 'second', start: 3, end: 5, text: 'visible', words: [{ start: 3, end: 4, text: 'visible' }] }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('first')
  await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('public secret'); await page.getByRole('button', { name: 'Merge next cue', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('public visible')
  await page.getByRole('button', { name: 'Save', exact: true }).click(); const cue = await page.evaluate(() => (window as any).__test.saved.edits.subtitles[0]); expect(cue.sourceRanges).toEqual([{ start: 0, end: .8 }, { start: 3, end: 4 }]); expect(cue.words.map((word: any) => word.text)).toEqual(['public', 'visible'])
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').some((call: any) => call.args.cue.text.includes('secret')))).toBe(false)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('first'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('public secret')
})

test('plain captions split at the text caret without inventing words and keyboard move shifts metadata', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { subtitles: [{ id: 'plain', start: 0, end: 4, text: 'First phrase second phrase', sourceRanges: [] }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('plain')
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '2'; element.dispatchEvent(new Event('input', { bubbles: true })) })
  await page.getByLabel('Subtitle text', { exact: true }).evaluate(element => { (element as HTMLTextAreaElement).setSelectionRange(12, 12); element.dispatchEvent(new Event('select', { bubbles: true })) })
  await page.getByRole('button', { name: 'Split at playhead', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('second phrase'); await expect(page.getByText('Split at the text caret. Both cues retain plain timing.')).toBeVisible()
  await page.getByRole('button', { name: 'Save', exact: true }).click(); expect(await page.evaluate(() => (window as any).__test.saved.edits.subtitles.map((cue: any) => cue.words))).toEqual([[], []])
  const second = page.getByRole('button', { name: /^Edit subtitles 2: second phrase,/ }); await second.focus(); await page.keyboard.press('Shift+ArrowRight'); await page.getByRole('button', { name: 'Save', exact: true }).click()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.subtitles[1])).toMatchObject({ start: 3, end: 5, sourceRanges: [{ start: 3, end: 5 }] })
})

test('word highlight caches bounded future appearances and shows plain fallback while its word loads', async ({ page }) => {
  await mockedDesktop(page)
  await page.addInitScript(() => { const w = window as any; w.__captionDraws = 0; const draw = CanvasRenderingContext2D.prototype.drawImage; CanvasRenderingContext2D.prototype.drawImage = function (image: CanvasImageSource, ...args: any[]) { if (image instanceof HTMLImageElement && this.canvas.classList.contains('preview-canvas')) w.__captionDraws++; return (draw as any).call(this, image, ...args) } as typeof draw })
  await page.goto('/'); await page.evaluate(() => { const t = (window as any).__test; t.holdSubtitleWord = true; t.patchProject({ edits: { subtitles: [{ id: 'words', start: 0, end: 6, text: 'one two three four five six seven eight nine ten', words: ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten'].map((text, index) => ({ text, start: index * .5, end: index * .5 + .4 })) }], subtitleStyle: { enabled: true, fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: true, highlightColor: '#E9B451' } } }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSubtitle)).toBe('function')
  await expect.poll(() => page.evaluate(() => (window as any).__captionDraws)).toBeGreaterThan(0)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').map((call: any) => call.args.wordIndex))).toEqual([null, 0])
  await page.evaluate(() => { const t = (window as any).__test; t.holdSubtitleWord = false; t.finishSubtitle() })
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)).toBe(7)
  await page.waitForTimeout(400); expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)).toBe(7)
  await page.getByLabel('Playhead position in seconds').evaluate(element => { (element as HTMLInputElement).value = '1.1'; element.dispatchEvent(new Event('input', { bubbles: true })) }); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview' && call.args.wordIndex === 2).length)).toBe(1)
  await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByLabel('Highlight spoken words').uncheck(); await page.getByLabel('Highlight spoken words').check(); await expect(page.getByRole('alert')).toHaveCount(0)
  await page.screenshot({ path: path.join(screenshotDir, 'stage5-subtitle-word-controls.png') })
})

test('local setup is explicit, completes without generation and local generation omits any cloud key', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const t = (window as any).__test; t.localStatus = { ready: false, runtimeReady: false, modelReady: false, runtimeVersion: '1.9.5', modelName: 'Whisper base multilingual', downloadBytes: 156880105, installedBytes: 0, error: null }; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click()
  await expect(page.getByLabel('Recognition provider')).toHaveValue('local'); await expect(page.getByLabel('Deepgram API key')).toHaveCount(0); await expect(page.getByRole('button', { name: 'Generate locally' })).toBeDisabled()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_local_transcription_install' || call.name === 'start_transcription'))).toBe(false)
  await page.getByRole('button', { name: 'Download local setup' }).click(); await expect(page.getByLabel('Local setup progress')).toHaveAttribute('value', '0.2')
  await page.evaluate(() => { const t = (window as any).__test; t.localStatus = { ...t.localStatus, ready: true, runtimeReady: true, modelReady: true, installedBytes: 156880105 }; t.localInstall = { ...t.localInstall, state: 'completed', progress: 1, status: t.localStatus }; t.emit('local-transcription-install-progress', t.localInstall) })
  await expect(page.getByRole('button', { name: 'Generate locally' })).toBeEnabled(); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_transcription'))).toBe(false)
  await page.screenshot({ path: path.join(screenshotDir, 'stage5-local-ready.png') })
  await page.getByRole('button', { name: 'Generate locally' }).click(); await expect(page.getByRole('dialog')).toBeHidden()
  const options = await page.evaluate(() => (window as any).__test.calls.find((call: any) => call.name === 'start_transcription').args.options); expect(options).toEqual({ provider: 'local', source: 'microphone', language: 'id' })
  await page.getByRole('button', { name: 'Cancel transcription', exact: true }).click()
})

test('closing local setup waits for a delayed start ACK and terminal cancellation before hiding', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const t = (window as any).__test; t.holdLocalStart = true; t.holdLocalCancel = true; t.localStatus = { ready: false, runtimeReady: false, modelReady: false, runtimeVersion: '1.9.5', modelName: 'Whisper base multilingual', downloadBytes: 156880105, installedBytes: 0, error: null }; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Download local setup' }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishLocalStart)).toBe('function'); await page.keyboard.press('Escape'); await expect(page.getByRole('dialog')).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'cancel_local_transcription_install'))).toBe(false)
  await page.evaluate(() => { const t = (window as any).__test; t.holdLocalStart = false; t.finishLocalStart() }); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishLocalCancel)).toBe('function'); await expect(page.getByRole('dialog')).toBeVisible()
  await page.evaluate(() => { const t = (window as any).__test; t.holdLocalCancel = false; t.finishLocalCancel() }); await expect(page.getByRole('dialog')).toBeHidden()
  const calls = await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name.includes('local_transcription_install'))); expect(calls.find((call: any) => call.name === 'cancel_local_transcription_install').args.jobId).toBe(calls.find((call: any) => call.name === 'start_local_transcription_install').args.jobId)
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_transcription'))).toBe(false)
})

test('local setup errors retry, reject stale completion and native close cancels the active job', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const t = (window as any).__test; t.localStatusError = 'Model storage is unavailable'; t.localStatus = { ready: false, runtimeReady: true, modelReady: false, runtimeVersion: '1.9.5', modelName: 'Whisper base multilingual', downloadBytes: 156880105, installedBytes: 10, error: null }; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.evaluate(() => { (window as any).__test.holdSave = true }); await page.getByLabel('Project name').fill('Unsaved setup project'); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click(); await expect(page.getByRole('alert')).toContainText('Model storage is unavailable')
  await page.evaluate(() => { (window as any).__test.localStatusError = null }); await page.getByRole('button', { name: 'Retry setup status' }).click(); await page.getByRole('button', { name: 'Download local setup' }).click()
  await expect(page.getByRole('button', { name: 'Cancel download' })).toBeEnabled()
  await page.evaluate(() => { const t = (window as any).__test; t.oldInstallJob = t.localInstall.jobId; t.localInstall = { ...t.localInstall, state: 'failed', error: 'Download checksum did not match.', status: null }; t.emit('local-transcription-install-progress', t.localInstall) }); await expect(page.getByRole('alert')).toContainText('checksum'); await page.getByRole('button', { name: 'Retry local setup' }).click()
  await page.evaluate(() => { const t = (window as any).__test; t.emit('local-transcription-install-progress', { ...t.localInstall, jobId: t.oldInstallJob, state: 'completed', status: { ...t.localStatus, ready: true } }) }); await expect(page.getByRole('button', { name: 'Generate locally' })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('close-blocked', 'Cancel local setup before closing.'))
  await expect(page.getByRole('dialog').locator('form')).toHaveAttribute('inert', ''); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishSave)).toBe('function')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(false)
  await page.evaluate(() => { const t = (window as any).__test; t.holdSave = false; t.finishSave() }); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(true)
  const names = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name)); expect(names.indexOf('cancel_local_transcription_install')).toBeLessThan(names.indexOf('close_app'))
})


test('timed caption cuts filter private words, Undo reuses safe glyphs and declared source gaps stay blank', async ({ page }) => {
  await mockedDesktop(page)
  await page.addInitScript(() => { const w = window as any; w.__captionDraws = 0; const draw = CanvasRenderingContext2D.prototype.drawImage; CanvasRenderingContext2D.prototype.drawImage = function (image: CanvasImageSource, ...args: any[]) { if (image instanceof HTMLImageElement && this.canvas.classList.contains('preview-canvas')) w.__captionDraws++; return (draw as any).call(this, image, ...args) } as typeof draw })
  await page.goto('/'); await page.evaluate(() => (window as any).__test.patchProject({ edits: { subtitles: [{ id: 'timed-private', start: 0, end: 6, text: 'safe secret retained', words: [{ start: 0, end: 1, text: 'safe' }, { start: 2, end: 3, text: 'secret' }, { start: 5, end: 6, text: 'retained' }], sourceRanges: [{ start: 0, end: 1 }, { start: 2, end: 3 }, { start: 5, end: 6 }] }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => (window as any).__captionDraws)).toBeGreaterThan(0)
  const seek = (value: number) => page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, value)
  await seek(1.5); await page.waitForTimeout(100); await page.evaluate(() => { (window as any).__captionDraws = 0 }); await seek(1.7); await page.waitForTimeout(100); expect(await page.evaluate(() => (window as any).__captionDraws)).toBe(0)
  await page.getByRole('button', { name: 'Cut range...', exact: true }).click(); await page.getByLabel('Output start (seconds)').fill('1.5'); await page.getByLabel('Output end (seconds)').fill('4'); await page.getByRole('button', { name: 'Cut range', exact: true }).click(); await seek(.5)
  await expect.poll(() => page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').map((call: any) => call.args.cue.text))).toEqual(['safe secret retained', 'safe retained'])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await seek(.5); await page.waitForTimeout(350)
  expect(await page.evaluate(() => (window as any).__test.calls.filter((call: any) => call.name === 'render_subtitle_preview').length)).toBe(2)
})

test('local setup dialog stays keyboard usable in a short light viewport and provider switch stops downloads', async ({ page }) => {
  await mockedDesktop(page); await page.setViewportSize({ width: 900, height: 640 }); await page.goto('/'); await page.getByRole('button', { name: 'Switch to light theme' }).click()
  await page.evaluate(() => { const t = (window as any).__test; t.localStatus = { ready: false, runtimeReady: false, modelReady: false, runtimeVersion: '1.9.5', modelName: 'Whisper base multilingual', downloadBytes: 156880105, installedBytes: 0, error: null }; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click()
  await page.getByRole('button', { name: 'Download local setup' }).focus(); await page.keyboard.press('Enter'); await expect(page.getByRole('button', { name: 'Cancel download' })).toBeEnabled()
  await page.getByLabel('Recognition provider').selectOption('cloud'); await expect(page.getByLabel('Deepgram API key')).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.localInstall.state)).toBe('cancelled')
  await page.getByLabel('Deepgram API key').focus(); await page.keyboard.press('Tab'); await page.screenshot({ path: path.join(screenshotDir, 'stage5-subtitle-cloud-light-900.png') })
  const bounds = (await page.getByRole('dialog').boundingBox())!; expect(bounds.x).toBeGreaterThanOrEqual(0); expect(bounds.y).toBeGreaterThanOrEqual(0); expect(bounds.y + bounds.height).toBeLessThanOrEqual(640)
  await page.keyboard.press('Escape'); await expect(page.getByRole('dialog')).toBeHidden(); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'start_transcription'))).toBe(false)
})


test('opening local setup cancels background analysis before installer status or download requests', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => { const t = (window as any).__test; t.holdAnalysis = true; t.patchProject({ microphonePath: '/test-media/audio.wav' }) })
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.rejectAnalysis)).toBe('function')
  await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByRole('button', { name: 'Generate subtitles', exact: true }).click(); await expect(page.getByRole('button', { name: 'Generate locally' })).toBeEnabled()
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name)); expect(calls.indexOf('cancel_audio_analysis')).toBeGreaterThan(0); expect(calls.indexOf('cancel_audio_analysis')).toBeLessThan(calls.indexOf('get_local_transcription_status'))
  await page.keyboard.press('Escape'); await expect(page.getByRole('dialog')).toBeHidden()
})


test('merging interleaved subtitle coverage saves chronological words and undo restores both cues', async ({ page }) => {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(() => (window as any).__test.patchProject({ edits: { subtitles: [{ id: 'outer', start: 0, end: 6, text: 'first last', words: [{ start: .2, end: .8, text: 'first' }, { start: 5.2, end: 5.8, text: 'last' }], sourceRanges: [{ start: 0, end: 1 }, { start: 5, end: 6 }] }, { id: 'inner', start: 2, end: 3, text: 'middle', words: [{ start: 2.2, end: 2.8, text: 'middle' }], sourceRanges: [{ start: 2, end: 3 }] }] } }))
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Subtitles', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('outer')
  await page.getByRole('button', { name: 'Merge next cue', exact: true }).click(); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('first middle last')
  await page.getByRole('button', { name: 'Save', exact: true }).click(); const merged = await page.evaluate(() => (window as any).__test.saved.edits.subtitles)
  expect(merged).toHaveLength(1); expect(merged[0].words.map((word: any) => word.start)).toEqual([.2, 2.2, 5.2]); expect(merged[0].sourceRanges).toEqual([{ start: .2, end: 1 }, { start: 2.2, end: 2.8 }, { start: 5, end: 5.8 }])
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Subtitle cue').selectOption('outer'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('first last'); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByLabel('Subtitle cue').selectOption('inner'); await expect(page.getByLabel('Subtitle text', { exact: true })).toHaveValue('middle')
})


async function openTrackedMask(page: Page, reviewed = false) {
  await mockedDesktop(page); await page.goto('/')
  await page.evaluate(reviewed => { const t = (window as any).__test; t.patchProject({ screenPath: '/test-media/cut-zones.mp4', cameraPath: null, width: 960, height: 540, duration: 6, edits: { trimEnd: 6, camera: { enabled: false, layout: 'bubble', x: .75, y: .64, width: .2, shape: 'circle', mirror: false }, zooms: [], padding: 0, cornerRadius: 0, showCursor: false, redactions: [{ id: 'tracked', start: 0, end: 6, x: .1, y: .2, width: .2, height: .2, mode: 'blur', strength: 12, tracking: { reviewed, keyframes: [{ time: 0, x: .1, y: .2, width: .2, height: .2, kind: 'manual', confidence: 1 }, { time: 1, x: .2, y: .2, width: .2, height: .2, kind: 'auto', confidence: .95 }, { time: 2, x: 0, y: 0, width: 1, height: 1, kind: 'lost', confidence: 0 }, { time: 4, x: .3, y: .3, width: .2, height: .2, kind: 'manual', confidence: 1 }] } }] } }) }, reviewed)
  await page.getByRole('button', { name: 'UI verification recording' }).click(); await page.getByRole('tab', { name: 'Masks', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
}
async function seekEditor(page: Page, seconds: number) {
  await page.getByLabel('Playhead position in seconds').evaluate((element, value) => { (element as HTMLInputElement).value = String(value); element.dispatchEvent(new Event('input', { bubbles: true })) }, seconds)
  try { await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden() }
  catch (error) {
    console.log('Seek failure diagnostics', JSON.stringify({ seconds, events: await page.evaluate(() => (window as any).__mediaClockEvents), media: await page.locator('video,audio').evaluateAll(elements => elements.map(element => { const media = element as HTMLMediaElement; return { track: media.dataset.track, src: media.currentSrc, currentTime: media.currentTime, duration: media.duration, readyState: media.readyState, seeking: media.seeking, paused: media.paused, error: media.error?.message } })) }))
    throw error
  }
}
async function centerPixel(page: Page) { return page.locator('canvas.preview-canvas').evaluate(canvas => { const c = canvas as HTMLCanvasElement; return Array.from(c.getContext('2d')!.getImageData(c.width * .7, c.height * .65, 1, 1).data).slice(0, 3) }) }

test('mask tracking drains analysis and save, ignores stale results, applies once and requires explicit review', async ({ page }) => {
  await openTrackedMask(page)
  await page.getByLabel('Project name').fill('Tracked draft'); await page.getByRole('button', { name: 'Track region again' }).click(); await expect(page.getByRole('button', { name: 'Cancel tracking', exact: true })).toBeEnabled(); await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  await page.evaluate(() => { const t = (window as any).__test; t.emit('mask-tracking-progress', { ...t.maskTracking, jobId: 'old', state: 'completed', tracking: { reviewed: true, keyframes: [] } }) }); await expect(page.locator('.workspace')).toHaveAttribute('inert', '')
  await page.evaluate(() => { const t = (window as any).__test; t.maskTracking = { ...t.maskTracking, state: 'completed', progress: 1, tracking: { reviewed: false, keyframes: [{ time: 0, x: .2, y: .2, width: .2, height: .2, kind: 'manual', confidence: 1 }, { time: 1, x: .3, y: .2, width: .2, height: .2, kind: 'auto', confidence: .9 }] } }; t.emit('mask-tracking-progress', t.maskTracking) })
  await expect(page.locator('.workspace')).not.toHaveAttribute('inert', ''); await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible(); await expect(page.getByLabel('Tracking keyframe')).toHaveJSProperty('length', 3)
  const calls = await page.evaluate(() => (window as any).__test.calls.map((call: any) => call.name)); expect(calls.indexOf('save_project_edits')).toBeLessThan(calls.indexOf('start_mask_tracking'))
  await page.getByRole('button', { name: 'Export MP4', exact: true }).click(); await expect(page.getByRole('dialog')).toBeHidden(); await expect(page.getByRole('alert')).toContainText('Review tracked masks')
  await page.getByRole('button', { name: 'Mark tracking reviewed' }).click(); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible(); await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByLabel('Tracking keyframe')).toHaveJSProperty('length', 5)
})

test('lost tracked blur masks the full screen and original correction exits safely on Escape, seek and tool change', async ({ page }) => {
  await openTrackedMask(page); await seekEditor(page, 2.5); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0]); await expect(page.getByText(/Tracking is lost here/)).toBeVisible()
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeDisabled(); await expect.poll(async () => (await centerPixel(page))[2]).toBeGreaterThan(220)
  await page.locator('canvas.preview-canvas').focus(); await page.keyboard.press('Space'); expect(await page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).paused)).toBe(true)
  await page.keyboard.press('Escape'); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0])
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await seekEditor(page, 2.7); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0])
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await page.getByRole('tab', { name: 'Audio', exact: true }).click(); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0])
})

test('drawing a correction adds one manual keyframe, restores masking and supports retrack and undo', async ({ page }) => {
  await openTrackedMask(page); await seekEditor(page, 2.5); await page.getByRole('button', { name: 'Draw correction on original' }).click()
  const box = (await page.locator('canvas.preview-canvas').boundingBox())!; await page.mouse.move(box.x + box.width * .2, box.y + box.height * .3); await page.mouse.down(); await page.mouse.move(box.x + box.width * .4, box.y + box.height * .5, { steps: 4 }); await page.mouse.up()
  await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect(page.getByRole('button', { name: 'Retrack from correction' })).toBeEnabled(); await page.getByRole('button', { name: 'Save', exact: true }).click()
  const keys = await page.evaluate(() => (window as any).__test.saved.edits.redactions[0].tracking.keyframes); expect(keys).toHaveLength(5); expect(keys[3]).toMatchObject({ time: 2.5, kind: 'manual', confidence: 1 }); expect(keys[3].x).toBeCloseTo(.2, 2); expect(keys[3].width).toBeCloseTo(.2, 2)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0]); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Redo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await page.getByRole('button', { name: 'Retrack from correction' }).click(); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.find((call: any) => call.name === 'start_mask_tracking')?.args.start)).toBe(2.5)
  await page.getByRole('button', { name: 'Cancel tracking', exact: true }).click(); await expect(page.locator('.workspace')).not.toHaveAttribute('inert', '')
})

test('tracked mask dragging and Escape restore reviewed metadata without an extra undo', async ({ page }) => {
  await openTrackedMask(page, true); await seekEditor(page, 4.5)
  const original = await page.evaluate(() => (window as any).__test.currentProject().edits.redactions[0]); const move = page.getByRole('button', { name: 'Move mask', exact: true }); const box = (await move.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2); await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2 + 15, { steps: 3 }); await page.keyboard.press('Escape'); await page.mouse.up()
  await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible(); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  expect(await page.evaluate(() => (window as any).__test.currentProject().edits.redactions[0])).toEqual(original)
  await move.focus(); await page.keyboard.press('Shift+ArrowRight'); await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible(); await page.getByRole('button', { name: 'Save', exact: true }).click()
  expect(await page.evaluate(() => (window as any).__test.saved.edits.redactions[0].tracking.keyframes.find((frame: any) => frame.time === 4.5)?.kind)).toBe('manual')
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible()
})

test('tracking keyframe deletion, numeric correction and clearing preserve undo and reject overlong ranges', async ({ page }) => {
  await openTrackedMask(page); await seekEditor(page, 4.5); await page.getByLabel('Region left %').fill('45'); await page.getByLabel('Region left %').blur(); await expect(page.getByRole('button', { name: 'Retrack from correction' })).toBeEnabled()
  await page.getByLabel('Tracking keyframe').selectOption('4.5'); await page.getByRole('button', { name: 'Delete keyframe' }).click(); await expect(page.getByLabel('Tracking keyframe')).toHaveJSProperty('length', 5)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByLabel('Tracking keyframe')).toHaveJSProperty('length', 6)
  await seekEditor(page, 2.5); await page.getByRole('button', { name: 'Clear tracking, keep current mask' }).click(); await expect(page.getByLabel('Effect', { exact: true })).toHaveValue('solid'); await expect(page.getByLabel('Region width %')).toHaveValue('100.00'); await expect(page.getByLabel('Tracking keyframe')).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await expect(page.getByLabel('Tracking keyframe')).toHaveJSProperty('length', 6)
})

test('native close during a pending tracking ACK waits for registration and terminal cancellation', async ({ page }) => {
  await openTrackedMask(page)
  await page.evaluate(() => { const t = (window as any).__test; t.holdMaskStart = true; t.holdMaskCancel = true }); await page.getByRole('button', { name: 'Track region again' }).click()
  await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishMaskStart)).toBe('function'); await expect(page.getByRole('button', { name: 'Cancel tracking', exact: true })).toBeDisabled()
  await page.evaluate(() => (window as any).__test.emit('close-blocked', 'Cancel tracking before closing.')); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'cancel_mask_tracking' || call.name === 'close_app'))).toBe(false)
  await page.evaluate(() => { const t = (window as any).__test; t.holdMaskStart = false; t.finishMaskStart() }); await expect.poll(() => page.evaluate(() => typeof (window as any).__test.finishMaskCancel)).toBe('function')
  expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(false)
  await page.evaluate(() => { const t = (window as any).__test; t.holdMaskCancel = false; t.finishMaskCancel() }); await expect.poll(() => page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'close_app'))).toBe(true)
  const calls = await page.evaluate(() => (window as any).__test.calls); expect(calls.find((call: any) => call.name === 'cancel_mask_tracking').args.jobId).toBe(calls.find((call: any) => call.name === 'start_mask_tracking').args.jobId)
})

test('aborting original-frame drawing and exporting or switching project never leaves raw pixels visible', async ({ page }) => {
  await openTrackedMask(page); await seekEditor(page, 2.5)
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await page.locator('canvas.preview-canvas').dispatchEvent('pointercancel', { pointerId: 1 }); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0])
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await page.getByRole('button', { name: 'Export MP4', exact: true }).click(); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0]); expect(await page.evaluate(() => (window as any).__test.calls.some((call: any) => call.name === 'export_project'))).toBe(false)
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); await page.getByRole('button', { name: 'UI verification recording' }).click(); await expect(page.getByText(/Original frame for correction/)).toBeHidden(); await expect(page.getByRole('button', { name: 'Play preview', exact: true })).toBeEnabled()
})


test('a mask correction uses the actual paused frame when playhead emissions are deliberately delayed', async ({ page }) => {
  await page.addInitScript(() => { const original = window.requestAnimationFrame; window.requestAnimationFrame = callback => original.call(window, () => callback(0)) })
  await openTrackedMask(page); await seekEditor(page, 4.2); await page.getByRole('button', { name: 'Play preview', exact: true }).click()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(4.45)
  await expect(page.getByLabel('Playhead position in seconds')).toHaveValue('4.2')
  await page.getByRole('button', { name: 'Draw correction on original' }).click(); const actualTime = await page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).currentTime)
  expect(Number(await page.getByLabel('Playhead position in seconds').inputValue())).toBeCloseTo(Math.round(actualTime * 30) / 30, 10)
  const box = (await page.locator('canvas.preview-canvas').boundingBox())!; await page.mouse.move(box.x + box.width * .2, box.y + box.height * .2); await page.mouse.down(); await page.mouse.move(box.x + box.width * .4, box.y + box.height * .4, { steps: 3 }); await page.mouse.up(); await page.getByRole('button', { name: 'Save', exact: true }).click()
  const keys = await page.evaluate(() => (window as any).__test.saved.edits.redactions[0].tracking.keyframes); expect(keys.at(-1).time).toBeCloseTo(actualTime, 10); expect(keys.at(-1).kind).toBe('manual')
})


test('tracking failures and oversized results preserve the previous mask and its review state', async ({ page }) => {
  await openTrackedMask(page, true); const before = await page.evaluate(() => (window as any).__test.currentProject().edits.redactions[0])
  await page.evaluate(() => { (window as any).__test.maskTrackingError = 'Tracker could not decode this source.' }); await page.getByRole('button', { name: 'Track region again' }).click(); await expect(page.getByRole('alert')).toContainText('could not decode'); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible()
  await page.evaluate(() => { (window as any).__test.maskTrackingError = null }); await page.getByRole('button', { name: 'Track region again' }).click(); await expect(page.getByRole('button', { name: 'Cancel tracking', exact: true })).toBeEnabled()
  await page.evaluate(() => { const t = (window as any).__test; t.maskTracking = { ...t.maskTracking, state: 'completed', progress: 1, tracking: { reviewed: false, keyframes: Array.from({ length: 3003 }, (_, i) => ({ time: i / 1000, x: .1, y: .1, width: .2, height: .2, confidence: .9, kind: i ? 'auto' : 'manual' })) } }; t.emit('mask-tracking-progress', t.maskTracking) }); await expect(page.getByRole('alert')).toContainText('3,002'); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__test.currentProject().edits.redactions[0])).toEqual(before); await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: 'Track region again' }).click(); await page.getByRole('button', { name: 'Cancel tracking', exact: true }).click(); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible(); expect(await page.evaluate(() => (window as any).__test.currentProject().edits.redactions[0])).toEqual(before)
})

test('tracked timing, effect edits and clear actions invalidate review and remain reachable in both themes', async ({ page }) => {
  await openTrackedMask(page, true); await page.setViewportSize({ width: 900, height: 640 }); await page.getByRole('button', { name: 'Projects', exact: true }).click(); await page.getByLabel('Effect', { exact: true }).selectOption('solid'); await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible(); await page.getByRole('button', { name: 'Mark tracking reviewed' }).click()
  await page.getByLabel('Region start (s)').fill('.5'); await page.getByLabel('Region start (s)').blur(); await expect(page.getByText('Tracking needs review', { exact: true })).toBeVisible(); await expect.poll(() => page.evaluate(() => (window as any).__test.saved?.edits.redactions[0].tracking.keyframes[0].time)).toBe(.5)
  await page.getByRole('button', { name: 'Mark tracking reviewed' }).focus(); await page.keyboard.press('Enter'); await expect(page.getByText('Tracking reviewed', { exact: true })).toBeVisible(); await page.screenshot({ path: path.join(screenshotDir, 'stage6-mask-tracking-dark-900.png') })
  await page.getByRole('button', { name: 'Switch to light theme' }).click(); await page.getByRole('button', { name: 'Clear tracking, keep current mask' }).focus(); await page.screenshot({ path: path.join(screenshotDir, 'stage6-mask-tracking-light-900.png') }); await page.keyboard.press('Enter'); await expect(page.getByLabel('Tracking keyframe')).toHaveCount(0)
  await page.getByRole('button', { name: 'Undo', exact: true }).click(); await page.getByLabel('Region', { exact: true }).selectOption('tracked'); await seekEditor(page, 2.5); await page.getByRole('button', { name: 'Draw correction on original' }).scrollIntoViewIfNeeded(); await page.screenshot({ path: path.join(screenshotDir, 'stage6-lost-mask-light-900.png') })
  await page.setViewportSize({ width: 1440, height: 900 }); await page.getByRole('button', { name: 'Draw correction on original' }).click(); await page.screenshot({ path: path.join(screenshotDir, 'stage6-original-correction-light-1440.png') }); await page.keyboard.press('Escape'); await expect.poll(() => centerPixel(page)).toEqual([0, 0, 0])
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth); expect(overflow).toBe(false)
})
