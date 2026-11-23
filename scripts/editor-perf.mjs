import { chromium } from '@playwright/test'
import { build } from 'vite'
import { createServer } from 'node:http'
import { createReadStream, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { createHash, randomUUID } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import path from 'node:path'
import os from 'node:os'

const root = path.resolve(import.meta.dirname, '..')
const runStartedAt = Date.now()
const artifacts = path.join(root, 'artifacts/perf-0.5.1')
const args = process.argv.slice(2)
const values = flag => args.flatMap((value, index) => value === flag ? [args[index + 1]] : [])
const rounds = Number(values('--rounds')[0] ?? 3)
const seconds = Number(values('--seconds')[0] ?? 6)
const output = path.resolve(values('--out')[0] ?? path.join(artifacts, 'measurements.json'))
const variants = values('--variant').map(value => { const index = value.indexOf('='); return { name: value.slice(0, index), source: path.resolve(value.slice(index + 1)) } })
if (!variants.length || variants.some(value => !/^[a-zA-Z0-9][a-zA-Z0-9.-]*$/.test(value.name) || value.name.includes('..') || !existsSync(path.join(value.source, 'src/App.vue'))) || new Set(variants.map(value => value.name)).size !== variants.length) throw new Error('Use distinct safe --variant name=source-snapshot-directory values.')
if (!(rounds >= 1 && rounds <= 10 && seconds >= 2 && seconds <= 8)) throw new Error('Rounds1..10 and seconds2..8 are required.')
if (!output.startsWith(artifacts + path.sep)) throw new Error('Benchmark output must remain inside artifacts/perf-0.5.1.')
mkdirSync(path.dirname(output), { recursive: true })
const runDirectory = path.join(artifacts, 'runs', `${Date.now()}-${randomUUID().slice(0, 8)}`)
mkdirSync(runDirectory, { recursive: true })
const media = path.join(artifacts, 'media')
mkdirSync(media, { recursive: true })
const ffmpeg = path.join(root, 'src-tauri/resources/ffmpeg.exe')
const sha = file => createHash('sha256').update(readFileSync(file)).digest('hex')
function hashes(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const relative = path.join(prefix, entry.name); const file = path.join(directory, entry.name)
    return entry.isDirectory() ? hashes(file, relative) : [{ path: relative.replaceAll('\\', '/'), bytes: statSync(file).size, sha256: sha(file) }]
  }).sort((a, b) => a.path.localeCompare(b.path))
}
for (const [name, filter, fps, bitrate] of [
  ['screen60.mp4', 'testsrc2=size=1920x1080:rate=60', 60, '12000k'],
  ['screen30.mp4', 'testsrc2=size=1920x1080:rate=30', 30, '12000k'],
  ['camera30.mp4', 'testsrc2=size=640x360:rate=30', 30, '2000k'],
]) {
  const file = path.join(media, name)
  if (!existsSync(file)) execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', filter, '-t', '12', '-c:v', 'h264_mf', '-hw_encoding', '0', '-b:v', bitrate, '-g', String(fps), '-pix_fmt', 'yuv420p', '-movflags', '+faststart', file], { windowsHide: true, timeout: 90_000, stdio: 'pipe' })
}
for (const variant of variants) {
  variant.hashes = hashes(path.join(variant.source, 'src'))
  variant.dist = path.resolve(runDirectory, variant.name)
  if (!variant.dist.startsWith(runDirectory + path.sep) || existsSync(variant.dist)) throw new Error('Build output must be a fresh directory inside the owned run directory.')
  await build({ root: variant.source, base: `/${variant.name}/`, configFile: path.join(variant.source, 'vite.config.ts'), logLevel: 'warn', build: { outDir: variant.dist, emptyOutDir: false } })
}

function projectFor(scenario) {
  const heavy = scenario === 'history'; const complex = scenario === 'playback60' || scenario === 'seek'
  return {
    version: 5, id: 'performance-fixture', name: 'Performance fixture', createdAt: '2026-11-19T00:00:00Z', directory: 'D:\\Synthetic-perf',
    screenPath: `/bench-media/screen${scenario === 'playback30' ? 30 : 60}.mp4`, cameraPath: complex ? '/bench-media/camera30.mp4' : null,
    microphonePath: null, systemAudioPath: null, cameraOffsetSeconds: 0, cameraIntervals: null,
    width: 1920, height: 1080, fps: scenario === 'playback30' ? 30 : 60, duration: 12, status: 'ready',
    cursor: Array.from({ length: 1501 }, (_, i) => ({ time: i / 125, x: .2 + .6 * i / 1500, y: .45 + .1 * Math.sin(i / 80), visible: true, pressed: i % 250 === 0 })),
    edits: {
      trimStart: .25, trimEnd: 11.5, videoClips: complex ? [{ id: 'a', start: .25, end: 2.5 }, { id: 'b', start: 3.5, end: 11.5 }] : [],
      aspectRatio: complex ? '9:16' : '16:9', fitMode: complex ? 'fill' : 'fit', background: '#283544', backgroundMode: complex ? 'videoBlur' : 'solid', backgroundBlur: 24,
      padding: 60, cornerRadius: 16, crop: null, showCursor: true, cursorSize: 1.5, cursorSmoothing: .5,
      reframeKeyframes: complex ? [{ id: 'one', time: 0, x: .3, y: .5, scale: 1 }, { id: 'two', time: 6, x: .7, y: .5, scale: 1.3 }, { id: 'three', time: 12, x: .4, y: .5, scale: 1 }] : [],
      zooms: complex ? [{ id: 'zoom', start: 4, end: 6, scale: 1.5, x: .5, y: .5, followCursor: true }] : [],
      camera: { enabled: complex, layout: 'bubble', x: .7, y: .7, width: .23, shape: 'circle', mirror: true },
      systemVolume: 1, microphoneVolume: 1, audioProcessing: { microphoneGain: 1, systemGain: 1, duckingEnabled: false, duckingAmountDb: 12 },
      redactions: complex ? [{ id: 'mask', start: 0, end: 12, x: .45, y: .4, width: .1, height: .15, mode: 'solid', strength: 12, tracking: null }] : [],
      music: [], subtitles: heavy ? Array.from({ length: 1000 }, (_, i) => ({ id: `cue-${i}`, start: i * .012, end: (i + 1) * .012, text: `Synthetic caption ${i} ${'bounded editor metadata '.repeat(8)}`, words: [], sourceRanges: [] })) : [],
      subtitleStyle: { enabled: false, fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: false, highlightColor: '#E9B451' },
    },
  }
}

function installInstrumentation(project) {
  window.isTauri = true
  localStorage.setItem('screenrecord-theme', 'dark'); localStorage.setItem('screenrecord-library', 'open')
  const callbacks = new Map(); const listeners = new Map(); let callbackId = 0; let saved = structuredClone(project)
  const state = { active: false, paused: false, phase: 'idle', cameraAvailable: false, elapsed: 0, projectId: null }
  const bench = { enabled: false, compositions: [], json: [], raf: [], rvfc: [], observedFrames: [], longTasks: [], saves: 0, errors: [], started: 0 }
  window.__perf = bench
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
    transformCallback: cb => { const id = ++callbackId; callbacks.set(id, cb); return id }, unregisterCallback: id => callbacks.delete(id), convertFileSrc: value => value,
    invoke: async (name, options) => {
      if (name === 'plugin:event|listen') { listeners.set(options.event, [...(listeners.get(options.event) ?? []), options.handler]); return options.handler }
      if (name === 'plugin:event|unlisten') return
      if (name === 'get_app_info') return { version: 'benchmark', projectsDirectory: 'Synthetic only', ffmpegAvailable: true, ffmpegPath: 'synthetic' }
      if (name === 'list_projects') return [structuredClone(saved)]
      if (name === 'open_project') return structuredClone(saved)
      if (name === 'get_recording_state') return state
      if (name === 'get_export_progress') return null
      if (name === 'get_stop_shortcut') return 'Ctrl+Shift+R'
      if (name === 'get_recovery_issues' || name === 'list_appearance_presets') return []
      if (name === 'save_project_edits') { saved = { ...saved, name: options.name, edits: structuredClone(options.edits) }; if (bench.enabled) { bench.saves++; bench.savedPadding = saved.edits.padding }; return { id: saved.id, name: saved.name, edits: structuredClone(saved.edits) } }
      if (name === 'cancel_audio_analysis' || name === 'close_app') return
      bench.errors.push(`Unexpected mocked command: ${name}`); throw new Error(`Unexpected mocked command: ${name}`)
    },
  }
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => callbacks.delete(id) }
  const stringify = JSON.stringify
  JSON.stringify = function(value, ...rest) {
    const tracked = bench.enabled && value && typeof value === 'object' && ('trimStart' in value || ('edits' in value && 'name' in value))
    if (!tracked) return stringify.call(this, value, ...rest)
    const start = performance.now(); const result = stringify.call(this, value, ...rest)
    bench.json.push({ milliseconds: performance.now() - start, bytes: result.length * 2 }); return result
  }
  const raf = window.requestAnimationFrame; const cancelRaf = window.cancelAnimationFrame
  const invokeRaf = (callback, time) => { const start = performance.now(); try { return callback(time) } finally { if (bench.enabled) bench.raf.push({ time: start, milliseconds: performance.now() - start }) } }
  if (project.fps === 30) {
    let pending = new Map(); let handle = 0; let id = 0; let deadline = 0
    const pump = time => {
      handle = 0
      if (time >= deadline) { if (!deadline || time - deadline > 1000 / 60) deadline = time; deadline += 1000 / 60; const current = pending; pending = new Map(); for (const callback of current.values()) invokeRaf(callback, time) }
      if (pending.size && !handle) handle = raf(pump)
    }
    window.requestAnimationFrame = callback => { const key = ++id; pending.set(key, callback); if (!handle) handle = raf(pump); return key }
    window.cancelAnimationFrame = key => { pending.delete(key); if (!pending.size && handle) { cancelRaf(handle); handle = 0 } }
  } else window.requestAnimationFrame = function(callback) { return raf.call(this, time => invokeRaf(callback, time)) }
  const rvfc = HTMLVideoElement.prototype.requestVideoFrameCallback
  if (rvfc) HTMLVideoElement.prototype.requestVideoFrameCallback = function(callback) { return rvfc.call(this, (time, metadata) => { const start = performance.now(); try { return callback(time, metadata) } finally { if (bench.enabled) bench.rvfc.push({ time: start, milliseconds: performance.now() - start }) } }) }
  const fill = CanvasRenderingContext2D.prototype.fillRect; const draw = CanvasRenderingContext2D.prototype.drawImage
  const active = new WeakMap()
  CanvasRenderingContext2D.prototype.fillRect = function(...params) {
    if (bench.enabled && this.canvas.classList.contains('preview-canvas')) {
      const video = document.querySelector('video[data-track="screen recording"]')
      active.set(this, { started: performance.now(), mediaTime: video?.currentTime ?? null, milliseconds: 0, drawCalls: 0 })
    }
    return fill.apply(this, params)
  }
  CanvasRenderingContext2D.prototype.drawImage = function(...params) {
    const result = draw.apply(this, params)
    if (bench.enabled && this.canvas.classList.contains('preview-canvas')) {
      const item = active.get(this)
      if (item) { if (!item.drawCalls) bench.compositions.push(item); item.drawCalls++; item.milliseconds = performance.now() - item.started }
    }
    return result
  }
  new PerformanceObserver(list => { if (bench.enabled) for (const item of list.getEntries()) bench.longTasks.push(item.duration) }).observe({ entryTypes: ['longtask'] })
  bench.reset = () => { for (const field of ['compositions', 'json', 'raf', 'rvfc', 'observedFrames', 'longTasks']) bench[field] = []; bench.saves = 0; bench.started = performance.now(); bench.enabled = true }
  bench.result = () => { bench.enabled = false; return { ...bench, elapsedMs: performance.now() - bench.started, reset: undefined, result: undefined, observe: undefined } }
  bench.observe = () => {
    const video = document.querySelector('video[data-track="screen recording"]')
    const next = (_time, metadata) => { if (bench.enabled) bench.observedFrames.push({ time: performance.now(), mediaTime: metadata.mediaTime, presentedFrames: metadata.presentedFrames }); if (video.isConnected) rvfc.call(video, next) }
    if (video && rvfc) rvfc.call(video, next)
  }
}

const quantile = (values, p) => { const sorted = [...values].sort((a, b) => a - b); return sorted.length ? sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * p) - 1)] : null }
function summary(samples) {
  const durations = samples.map(value => value.milliseconds)
  return { count: samples.length, totalMs: durations.reduce((sum, value) => sum + value, 0), p50Ms: quantile(durations, .5), p95Ms: quantile(durations, .95) }
}
function summarize(value) {
  const seek = value.seeks ?? []
  return { elapsedMs: value.elapsedMs, compositions: summary(value.compositions), observedFrames: value.observedFrames.length, json: { ...summary(value.json), totalBytes: value.json.reduce((sum, item) => sum + item.bytes, 0) }, raf: { ...summary(value.raf), intervalP50Ms: quantile(value.raf.slice(1).map((item, i) => item.time - value.raf[i].time), .5) }, rvfc: summary(value.rvfc), longTasks: { count: value.longTasks.length, totalMs: value.longTasks.reduce((a, b) => a + b, 0), maxMs: Math.max(0, ...value.longTasks) }, saves: value.saves, cdp: value.cdp, quality: value.quality, seeks: { count: seek.length, p50Ms: quantile(seek.map(item => item.milliseconds), .5), p95Ms: quantile(seek.map(item => item.milliseconds), .95) }, gestureMs: value.gestureMs ?? null, errors: value.errors }
}
function aggregate(runs) {
  const groups = new Map()
  for (const run of runs) { const key = `${run.variant}/${run.scenario}`; if (!groups.has(key)) groups.set(key, []); groups.get(key).push(run.summary) }
  return Object.fromEntries([...groups].map(([key, samples]) => [key, {
    repetitions: samples.length,
    medianElapsedMs: quantile(samples.map(s => s.elapsedMs), .5),
    medianCompositions: quantile(samples.map(s => s.compositions.count), .5),
    medianPresentedFrames: quantile(samples.map(s => s.observedFrames), .5),
    medianTotalFrames: quantile(samples.map(s => s.quality.totalFrames), .5),
    medianCompositionP50Ms: quantile(samples.map(s => s.compositions.p50Ms), .5),
    medianCompositionP95Ms: quantile(samples.map(s => s.compositions.p95Ms), .5),
    medianTaskMs: quantile(samples.map(s => s.cdp.TaskDurationMs), .5),
    medianScriptMs: quantile(samples.map(s => s.cdp.ScriptDurationMs), .5),
    medianJsonCount: quantile(samples.map(s => s.json.count), .5),
    medianJsonMs: quantile(samples.map(s => s.json.totalMs), .5),
    medianJsonBytes: quantile(samples.map(s => s.json.totalBytes), .5),
    medianGestureMs: quantile(samples.map(s => s.gestureMs).filter(s => s !== null), .5),
    medianJsHeapUsedBefore: quantile(samples.map(s => s.cdp.jsHeapUsedBytesBefore), .5),
    medianJsHeapUsedAfter: quantile(samples.map(s => s.cdp.jsHeapUsedBytesAfter), .5),
    pooledSeekP50Ms: quantile(runs.filter(r => `${r.variant}/${r.scenario}` === key).flatMap(r => (r.raw.seeks ?? []).map(s => s.milliseconds)), .5),
    pooledSeekP95Ms: quantile(runs.filter(r => `${r.variant}/${r.scenario}` === key).flatMap(r => (r.raw.seeks ?? []).map(s => s.milliseconds)), .95),
    droppedFrames: samples.map(s => s.quality.droppedFrames),
  }]))
}
const server = createServer((request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1'); const parts = decodeURIComponent(url.pathname).split('/').filter(Boolean)
  const base = parts[0] === 'bench-media' ? media : variants.find(value => value.name === parts[0])?.dist
  if (!base) { response.writeHead(404); response.end(); return }
  const file = path.resolve(base, ...parts.slice(1), ...(parts.length === 1 ? ['index.html'] : []))
  if (!file.startsWith(base + path.sep) || !existsSync(file) || !statSync(file).isFile()) { response.writeHead(404); response.end(); return }
  const size = statSync(file).size; const match = request.headers.range?.match(/bytes=(\d+)-(\d*)/); const start = Number(match?.[1] ?? 0); const end = Math.min(Number(match?.[2] || size - 1), size - 1)
  const type = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.mp4': 'video/mp4' }[path.extname(file)] ?? 'application/octet-stream'
  response.writeHead(match ? 206 : 200, { 'Content-Type': type, 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes', ...(match ? { 'Content-Range': `bytes ${start}-${end}/${size}` } : {}) })
  createReadStream(file, { start, end }).pipe(response)
})
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const address = `http://127.0.0.1:${server.address().port}`
let browser
try { browser = await chromium.launch({ executablePath: process.env.BROWSER_PATH ?? 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe', headless: true, args: ['--mute-audio', '--disable-background-timer-throttling', '--disable-renderer-backgrounding'] }) }
catch (error) { await new Promise(resolve => server.close(resolve)); throw error }
const report = { methodology: { harnessSha256: sha(import.meta.filename), rounds, seconds, browser: browser.version(), node: process.version, platform: `${os.platform()} ${os.release()}`, cpu: os.cpus()[0]?.model, memoryBytes: os.totalmem(), viewport: { width: 1440, height: 900 }, instrumentation: 'Unmodified production bundles; preview-canvas fillRect+drawImage composition spans, RAF/rVFC callback wall times, edits JSON.stringify, CDP task/script time and JS heap. Playback30 only uses a harness60Hz RAF budget with native phase jitter; other scenarios use actual native refresh. GPU time/native RAM are not measured. Synthetic local media; mocked IPC, no native capture.', scenarios: ['playback60:1080p60,portraitBlur,camera30,cursor,solidMask,sourceCut,reframe,nativeRefresh', 'playback30:1080p30,60HzRafBudget,landscapeSolid,cursor,noCamera', 'history:paused1080p60,1000captions,60paddingInputSteps,oneGesture,autosave', 'seek:6editedTimelineTargets,inclusiveCutBoundary,decodedFrameAndCompositionConfirmation'] }, variants: variants.map(({ name, source, hashes }) => ({ name, source, hashes })), media: hashes(media), runs: [] }
try {
  for (let round = 0; round < rounds; round++) for (const variant of (round % 2 ? [...variants].reverse() : variants)) for (const scenario of ['playback60', 'playback30', 'history', 'seek']) {
    const context = await browser.newContext({ viewport: report.methodology.viewport }); const page = await context.newPage()
    const pageErrors = []; page.on('pageerror', error => pageErrors.push(error.message))
    await context.route('**/*', route => route.request().url().startsWith(address) ? route.continue() : route.abort())
    await page.addInitScript(installInstrumentation, projectFor(scenario))
    await page.goto(`${address}/${variant.name}/`)
    await page.locator('.project-open').filter({ hasText: 'Performance fixture' }).click()
    await page.waitForFunction(() => { const video = document.querySelector('video[data-track="screen recording"]'); return video?.readyState >= 2 && !video.seeking && !document.querySelector('.preview-loading') })
    await page.waitForTimeout(600)
    const cdp = await context.newCDPSession(page); await cdp.send('Performance.enable')
    await page.evaluate(() => { window.__perf.observe(); window.__perf.reset() })
    const before = (await cdp.send('Performance.getMetrics')).metrics
    const qualityBefore = await page.evaluate(() => { const q = document.querySelector('video[data-track="screen recording"]').getVideoPlaybackQuality(); return { total: q.totalVideoFrames, dropped: q.droppedVideoFrames } })
    if (scenario.startsWith('playback')) { await page.getByRole('button', { name: 'Play preview' }).click(); await page.waitForTimeout(seconds * 1000); await page.getByRole('button', { name: 'Pause preview' }).click() }
    else if (scenario === 'history') {
      await page.locator('#padding').evaluate(async input => {
        const started = performance.now(); input.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }))
        for (let step = 0; step < 60; step++) { await new Promise(requestAnimationFrame); input.value = String(40 + step * 2); input.dispatchEvent(new Event('input', { bubbles: true })) }
        input.dispatchEvent(new Event('change', { bubbles: true })); input.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 })); input.blur(); window.__perf.gestureMs = performance.now() - started
      })
      await page.waitForTimeout(1100)
      const afterGesture = Number(await page.locator('#padding').inputValue())
      await page.getByRole('button', { name: 'Undo', exact: true }).click()
      const afterUndo = Number(await page.locator('#padding').inputValue())
      await page.getByRole('button', { name: 'Redo', exact: true }).click()
      const afterRedo = Number(await page.locator('#padding').inputValue())
      await page.waitForTimeout(900)
      const saved = await page.evaluate(() => window.__perf.savedPadding)
      if (afterGesture !== 158 || afterUndo !== 60 || afterRedo !== 158 || saved !== 158) throw new Error(`History contract failed: gesture=${afterGesture}, undo=${afterUndo}, redo=${afterRedo}, saved=${saved}`)
      await page.evaluate(values => { window.__perf.historyValues = values }, { afterGesture, afterUndo, afterRedo, saved })
    } else {
      await page.getByLabel('Playhead position in seconds').evaluate(async input => {
        window.__perf.seeks = []
        for (const output of [.5, 2.2, 2.25, 3.8, 1, 7.2]) {
          const source = output < 2.25 ? .25 + output : 3.5 + output - 2.25
          const began = performance.now(); input.value = String(output); input.dispatchEvent(new Event('input', { bubbles: true }))
          while (true) {
            const video = document.querySelector('video[data-track="screen recording"]'); const frame = window.__perf.compositions.at(-1); const decoded = window.__perf.observedFrames.at(-1)
            if (!video.seeking && !document.querySelector('.preview-loading') && frame?.started >= began && Math.abs(frame.mediaTime - source) < .002 && decoded?.time >= began && Math.abs(decoded.mediaTime - source) <= 1 / 60 + .002) break
            if (performance.now() - began > 5000) throw new Error(`Seek did not produce corresponding decoded composition: ${output} -> ${source}`)
            await new Promise(resolve => setTimeout(resolve, 5))
          }
          window.__perf.seeks.push({ output, source, milliseconds: performance.now() - began })
        }
      })
    }
    const after = (await cdp.send('Performance.getMetrics')).metrics
    const result = await page.evaluate(() => window.__perf.result())
    const qualityAfter = await page.evaluate(() => { const q = document.querySelector('video[data-track="screen recording"]').getVideoPlaybackQuality(); return { total: q.totalVideoFrames, dropped: q.droppedVideoFrames } })
    result.quality = { totalFrames: qualityAfter.total - qualityBefore.total, droppedFrames: qualityAfter.dropped - qualityBefore.dropped }
    result.errors.push(...pageErrors); result.cdp = Object.fromEntries(['TaskDuration', 'ScriptDuration', 'LayoutDuration', 'RecalcStyleDuration'].map(name => [name + 'Ms', 1000 * (after.find(item => item.name === name).value - before.find(item => item.name === name).value)]))
    result.cdp.jsHeapUsedBytesBefore = before.find(item => item.name === 'JSHeapUsedSize')?.value ?? null; result.cdp.jsHeapUsedBytesAfter = after.find(item => item.name === 'JSHeapUsedSize')?.value ?? null
    const run = { variant: variant.name, round, scenario, summary: summarize(result), raw: result }
    report.runs.push(run); report.aggregates = aggregate(report.runs); writeFileSync(output, JSON.stringify(report, null, 2)); process.stdout.write(JSON.stringify({ variant: run.variant, round, scenario, ...run.summary }) + '\n')
    await context.close()
    if (result.errors.length) throw new Error(`Benchmark application errors: ${result.errors.join('; ')}`)
  }
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)) }
report.runTiming = { startedAt: new Date(runStartedAt).toISOString(), completedAt: new Date().toISOString(), elapsedMs: Date.now() - runStartedAt }
writeFileSync(output, JSON.stringify(report, null, 2))
