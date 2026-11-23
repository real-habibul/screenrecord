import { chromium } from '@playwright/test'
import { build, preview } from 'vite'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const evidence = path.join(root, 'artifacts', 'recording-0.6.0')
const sources = process.argv.slice(2)
if (!sources.length || sources.length > 2) throw new Error('Pass one or two source directories in comparison order.')
await mkdir(evidence, { recursive: true })
const directory = await mkdtemp(path.join(evidence, 'setup-perf-'))
const browser = await chromium.launch({
  executablePath: process.env.BROWSER_PATH ?? 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  headless: true, args: ['--mute-audio', '--disable-renderer-backgrounding', '--disable-background-timer-throttling'],
})
const variants = []

function mockDesktop() {
  const idle = { active: false, paused: false, phase: 'idle', elapsed: 0, projectId: null, cameraAvailable: false }
  const monitor = { id: 'monitor:1', kind: 'monitor', title: 'Benchmark display', width: 1920, height: 1080 }
  const fallback = { sources: [monitor], microphones: [], cameras: [], apps: [] }
  let callback = 0
  window.isTauri = true
  const attempts = []
  window.__captureAttempts = attempts
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
    transformCallback: () => ++callback, unregisterCallback: () => {}, convertFileSrc: value => value,
    invoke: async name => {
      switch (name) {
        case 'plugin:event|listen': return ++callback
        case 'plugin:event|unlisten': return
        case 'get_app_info': return { version: 'benchmark', projectsDirectory: '', ffmpegAvailable: true }
        case 'list_projects': case 'get_recovery_issues': case 'list_appearance_presets': return []
        case 'get_recording_state': return idle
        case 'get_export_progress': return null
        case 'get_stop_shortcut': return 'Ctrl+Shift+R'
        case 'list_capture_devices': return fallback
        default: attempts.push(name); throw new Error('Unexpected IPC: ' + name)
      }
    },
  }
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
}

async function measure(page) {
  const visibleOpener = page.getByRole('button', { name: 'New recording', exact: true }).first()
  await visibleOpener.waitFor()
  const elapsed = await visibleOpener.evaluate(button => new Promise((resolve, reject) => {
    const started = performance.now()
    const observer = new MutationObserver(check)
    const timer = setTimeout(() => { observer.disconnect(); reject(new Error('Setup did not become ready')) }, 5000)
    function check() {
      const start = [...document.querySelectorAll('button')].find(el => el.textContent.trim() === 'Start recording' && !el.disabled)
      if (!start || !start.getClientRects().length) return
      clearTimeout(timer)
      observer.disconnect()
      resolve(performance.now() - started)
    }
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true })
    button.click()
    check()
  }))
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Start recording', exact: true }).waitFor({ state: 'detached' })
  return elapsed
}

try {
  for (const [index, sourceDir] of sources.entries()) {
    const source = path.resolve(sourceDir)
    const outDir = path.join(directory, String(index))
    const configFile = path.join(source, 'vite.config.ts')
    await build({ root: source, configFile, logLevel: 'warn', build: { outDir, emptyOutDir: false } })
    const server = await preview({ root: source, configFile, build: { outDir }, preview: { host: '127.0.0.1', port: 0, strictPort: false } })
    const page = await browser.newPage({ viewport: { width: 1380, height: 880 } })
    variants.push({ source, server, page, samples: [] })
    await page.addInitScript(mockDesktop)
    await page.goto('http://127.0.0.1:' + server.httpServer.address().port)
  }
  for (let round = 0; round < 27; round++) {
    const order = round % 2 ? [...variants].reverse() : variants
    for (const variant of order) {
      const elapsed = await measure(variant.page)
      if (round >= 3) variant.samples.push(elapsed)
    }
  }
  const results = []
  for (const { source, page, samples } of variants) {
    const sorted = [...samples].sort((a, b) => a - b)
    const unexpected = await page.evaluate(() => window.__captureAttempts)
    if (unexpected.length) throw new Error('Unexpected IPC: ' + unexpected.join(', '))
    results.push({ source, samples, p50Ms: sorted[Math.ceil(sorted.length * .5) - 1], p95Ms: sorted[Math.ceil(sorted.length * .95) - 1], unexpected })
  }
  const report = { recordedAt: new Date().toISOString(), chrome: browser.version(), method: 'Production build, mocked native IPC, click to enabled Start button with layout. Three warmups, 24 samples per variant. No device or screen access.', results }
  await writeFile(path.join(directory, 'measurements.json'), JSON.stringify(report, null, 2))
  console.log(JSON.stringify({ directory, results: results.map(({ samples, ...value }) => value) }))
} finally {
  await browser.close()
  await Promise.all(variants.map(({ server }) => server.close()))
}
