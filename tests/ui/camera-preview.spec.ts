import { expect, test, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import path from 'node:path'

let jpeg: string
test.beforeAll(() => {
  jpeg = execFileSync(process.env.SCREENRECORD_FFMPEG ?? path.resolve('src-tauri/resources/ffmpeg.exe'), ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=blue:size=480x270', '-frames:v', '1', '-c:v', 'mjpeg', '-threads', '1', '-f', 'image2pipe', 'pipe:1'], { windowsHide: true }).toString('base64')
})

async function mountPreview(page: Page) {
  const compiled = await (await page.request.get('/src/components/CameraPreview.vue')).text()
  const vueImport = compiled.match(/from\s+["']([^"']*\/vue\.js[^"']*)["']/)?.[1]
  if (!vueImport) throw new Error('The preview harness could not resolve the component Vue runtime.')
  await page.addInitScript(({ data }) => {
    const w = window as any
    w.isTauri = true
    w.__preview = { calls: [], active: false, holdStart: false, releaseStart: null, holdFrame: false, releaseFrame: null, failure: null, sequence: 0 }
    w.__TAURI_INTERNALS__ = { invoke: async (name: string, args: any) => {
      const state = w.__preview
      state.calls.push({ name, args })
      if (name === 'start_camera_preview') {
        if (state.holdStart) await new Promise<void>(resolve => { state.releaseStart = resolve })
        state.active = true
        return
      }
      if (name === 'stop_camera_preview') { state.active = false; return }
      if (name === 'get_camera_preview_frame') {
        const frame = { sequence: ++state.sequence, mimeType: 'image/jpeg', data, active: state.active, error: state.failure }
        if (state.holdFrame) await new Promise<void>(resolve => { state.releaseFrame = resolve })
        return frame
      }
      throw new Error(`Unexpected preview command: ${name}`)
    } }
  }, { data: jpeg })
  await page.route('**/preview-harness', route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><html lang="en"><head><title>Camera preview test</title><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/src/style.css"></head><body><main id="preview-root" style="width:min(340px,100%);padding:16px"></main><script type="module">
    import { createApp, reactive, h } from '${vueImport}';
    import CameraPreview from '/src/components/CameraPreview.vue';
    const props = reactive({ cameraId: '@device_native_camera_1', disabled: false, active: true });
    window.__preview.props = props;
    const app = createApp({ render: () => h(CameraPreview, { ...props, ref: value => { window.__preview.component = value } }) });
    app.mount('#preview-root'); window.__preview.unmount = () => app.unmount();
  </script></body></html>` }))
  await page.goto('/preview-harness')
  await expect(page.getByRole('button', { name: 'Enable preview' })).toBeVisible()
}

test('camera preview requires explicit action, displays JPEG and releases on stop, selection, disable and unmount', async ({ page }) => {
  await mountPreview(page)
  expect(await page.evaluate(() => (window as any).__preview.calls)).toHaveLength(0)
  await page.getByRole('button', { name: 'Enable preview' }).focus()
  await page.keyboard.press('Enter')
  const frame = page.getByRole('img', { name: 'Live preview of the selected camera' })
  await expect(frame).toBeVisible()
  await expect.poll(() => frame.evaluate(image => (image as HTMLImageElement).naturalWidth)).toBe(480)
  const start = await page.evaluate(() => (window as any).__preview.calls[0])
  expect(start.args.cameraId).toBe('@device_native_camera_1')
  expect(start.args.sessionId).toMatch(/^[\da-f-]{36}$/)
  await page.getByRole('button', { name: 'Stop preview' }).click()
  await expect(frame).toHaveCount(0)
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect(frame).toBeVisible()
  await page.evaluate(() => { (window as any).__preview.props.cameraId = '@device_native_camera_2' })
  await expect(page.getByRole('button', { name: 'Enable preview' })).toBeVisible()
  const starts = await page.evaluate(() => (window as any).__preview.calls.filter((c: any) => c.name === 'start_camera_preview').length)
  expect(starts).toBe(2)
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect(frame).toBeVisible()
  await page.evaluate(() => { (window as any).__preview.props.disabled = true })
  await expect(page.getByRole('button', { name: 'Enable preview' })).toBeDisabled()
  await expect(frame).toHaveCount(0)
  await page.evaluate(() => { (window as any).__preview.props.disabled = false })
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect(frame).toBeVisible()
  await page.evaluate(() => (window as any).__preview.unmount())
  await expect.poll(() => page.evaluate(() => (window as any).__preview.active)).toBe(false)
})

test('late startup is stopped after acknowledgement and cannot resurrect preview', async ({ page }) => {
  await mountPreview(page)
  await page.evaluate(() => { (window as any).__preview.holdStart = true })
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect(page.getByText('Opening camera...')).toBeVisible()
  await page.getByRole('button', { name: 'Stop preview' }).click()
  await expect(page.getByText('Releasing camera...')).toBeVisible()
  await page.evaluate(() => (window as any).__preview.releaseStart())
  await expect(page.getByRole('button', { name: 'Enable preview' })).toBeVisible()
  expect(await page.evaluate(() => (window as any).__preview.active)).toBe(false)
  expect(await page.evaluate(() => (window as any).__preview.calls.filter((c: any) => c.name === 'get_camera_preview_frame'))).toHaveLength(0)
})

test('late frames are discarded and device errors expose a working retry', async ({ page }) => {
  await mountPreview(page)
  await page.evaluate(() => { (window as any).__preview.holdFrame = true })
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect.poll(() => page.evaluate(() => !!(window as any).__preview.releaseFrame)).toBe(true)
  await page.evaluate(async () => { await (window as any).__preview.component.stop(); (window as any).__preview.releaseFrame() })
  await expect(page.getByRole('img')).toHaveCount(0)
  await page.evaluate(() => { const state = (window as any).__preview; state.holdFrame = false; state.failure = 'Camera disconnected.' })
  await page.getByRole('button', { name: 'Enable preview' }).click()
  await expect(page.getByRole('alert')).toHaveText('Camera disconnected.')
  expect(await page.evaluate(() => (window as any).__preview.active)).toBe(false)
  await page.evaluate(() => { (window as any).__preview.failure = null })
  await page.getByRole('button', { name: 'Retry preview' }).click()
  await expect(page.getByRole('img')).toBeVisible()
  await page.evaluate(() => { (window as any).__preview.active = false })
  await expect(page.getByRole('alert')).toContainText('Camera preview ended')
  await page.getByRole('button', { name: 'Retry preview' }).click()
  await expect(page.getByRole('img')).toBeVisible()
  await page.evaluate(async () => { await (window as any).__preview.component.stop(); (window as any).__preview.props.cameraId = null })
  await expect(page.getByRole('button', { name: 'Enable preview' })).toBeDisabled()
  await expect(page.getByText('Camera is off')).toBeVisible()
})

test('preview controls fit narrow layouts in both themes and keep keyboard focus visible', async ({ page }) => {
  await mountPreview(page)
  await page.setViewportSize({ width: 320, height: 600 })
  for (const theme of ['dark', 'light']) {
    await page.evaluate(value => { document.documentElement.dataset.theme = value }, theme)
    const button = page.getByRole('button', { name: 'Enable preview' })
    await page.keyboard.press('Tab')
    await button.focus()
    expect(await button.evaluate(el => getComputedStyle(el).outlineStyle)).not.toBe('none')
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    await button.press('Space')
    await expect(page.getByRole('img')).toBeVisible()
    await page.getByRole('button', { name: 'Stop preview' }).click()
  }
})
