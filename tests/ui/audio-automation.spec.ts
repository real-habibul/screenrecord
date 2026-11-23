import { expect, test } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'

const directory = path.resolve('artifacts/volume-stage4')
const audioFixture = path.join(directory, 'constant.wav')
const videoFixture = path.join(directory, 'screen.mp4')
test.beforeAll(() => {
  mkdirSync(directory, { recursive: true })
  const bundled = path.resolve('src-tauri/resources/ffmpeg.exe'); const engine = process.env.SCREENRECORD_FFMPEG ?? (existsSync(bundled) ? bundled : 'ffmpeg')
  if (!existsSync(audioFixture)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'aevalsrc=0.6|0.6:s=48000', '-t', '6', '-c:a', 'pcm_f32le', audioFixture], { windowsHide: true })
  if (!existsSync(videoFixture)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=black:s=320x180:r=30:d=6', ...(process.platform === 'win32' ? ['-c:v', 'h264_mf', '-hw_encoding', '0', '-b:v', '1000k'] : ['-c:v', 'libx264', '-preset', 'ultrafast']), '-pix_fmt', 'yuv420p', '-movflags', '+faststart', videoFixture], { windowsHide: true })
})

for (const duckedSolo of [false, true]) test(`sample-scheduled source volume ramps cross fractional cuts${duckedSolo ? ' before the solo computer speech detector' : ' on all three buses'}`, async ({ page }) => {
  await page.goto('/')
  const result = await page.evaluate(async duckedSolo => {
    const { scheduleVolume } = await import('/src/preview-volume.ts')
    const { MusicDucker } = await import('/audio-ducker.js')
    const rate = 48000; const first = 24000; const second = 12000; const gap = 12800
    const context = new OfflineAudioContext(2, first + gap + second, rate)
    await context.audioWorklet.addModule('/audio-ducker.js')
    const mixer = new AudioWorkletNode(context, 'screenrecord-music-mixer', { numberOfInputs: 3, numberOfOutputs: 1, outputChannelCount: [2], channelCount: 2, channelCountMode: 'explicit' })
    mixer.port.postMessage({ enabled: duckedSolo, systemEnabled: duckedSolo, amountDb: 12 }); mixer.port.postMessage({ monitor: duckedSolo ? 'system' : 'mix' }); mixer.connect(context.destination)
    const active = mixer.parameters.get('active')!; active.setValueAtTime(1, 0); active.setValueAtTime(0, first / rate); active.setValueAtTime(1, (first + gap) / rate)
    const names = ['microphone', 'system', 'music']; const levels = [[1, 0], [0, 1], [.25, .75]]
    for (const [index, bus] of names.entries()) {
      const keys = [{ id: 'a', time: 0, volume: levels[index][0] }, { id: 'b', time: 2, volume: levels[index][1] }]
      const param = mixer.parameters.get(`${bus}Volume`)!
      scheduleVolume(param, keys, .123456, .623456, 0)
      scheduleVolume(param, keys, 1.234567, 1.484567, (first + gap) / rate)
      const buffer = context.createBuffer(2, first + gap + second, rate)
      for (let frame = 0; frame < buffer.length; frame++) { const value = frame >= first && frame < first + gap ? 1 : [.2, .15, .3][index]; buffer.getChannelData(0)[frame] = value; buffer.getChannelData(1)[frame] = -value }
      const source = context.createBufferSource(); source.buffer = buffer; source.connect(mixer, 0, index); source.start()
    }
    const rendered = await context.startRendering(); const detector = new MusicDucker(rate); detector.configure(duckedSolo, 12)
    const parameterContext = new OfflineAudioContext(3, first + gap + second, rate); const merger = parameterContext.createChannelMerger(3); merger.connect(parameterContext.destination)
    for (let index = 0; index < 3; index++) {
      const source = parameterContext.createConstantSource(); source.offset.value = 1; const gain = parameterContext.createGain(); source.connect(gain); gain.connect(merger, 0, index)
      const keys = [{ id: 'a', time: 0, volume: levels[index][0] }, { id: 'b', time: 2, volume: levels[index][1] }]
      scheduleVolume(gain.gain, keys, .123456, .623456, 0); scheduleVolume(gain.gain, keys, 1.234567, 1.484567, (first + gap) / rate); source.start()
    }
    const nativeParameters = await parameterContext.startRendering(); const nativeDetector = new MusicDucker(rate); nativeDetector.configure(duckedSolo, 12)
    let error = 0; let engineMixError = 0; let parameterError = 0; let removedPeak = 0; let positivePeak = 0; let worstFrame = 0
    for (let frame = 0; frame < first + gap + second; frame++) {
      if (frame >= first && frame < first + gap) { removedPeak = Math.max(removedPeak, Math.abs(rendered.getChannelData(0)[frame])); continue }
      const time = frame < first ? .123456 + frame / rate : 1.234567 + (frame - first - gap) / rate
      const mic = Math.fround(Math.fround(.2) * Math.fround(1 - time / 2)); const system = Math.fround(Math.fround(.15) * Math.fround(time / 2)); const music = Math.fround(Math.fround(.3) * Math.fround(.25 + time / 4))
      const gain = detector.process(mic, -mic); const expected = Math.fround(duckedSolo ? system * gain : mic + system + music)
      const nativeMic = Math.fround(Math.fround(.2) * nativeParameters.getChannelData(0)[frame]); const nativeSystem = Math.fround(Math.fround(.15) * nativeParameters.getChannelData(1)[frame]); const nativeMusic = Math.fround(Math.fround(.3) * nativeParameters.getChannelData(2)[frame]); const nativeGain = nativeDetector.process(nativeMic, -nativeMic)
      const nativeExpected = Math.fround(duckedSolo ? nativeSystem * nativeGain : nativeMic + nativeSystem + nativeMusic)
      for (let channel = 0; channel < 2; channel++) { const delta = Math.abs(rendered.getChannelData(channel)[frame] - (channel ? -expected : expected)); if (delta > error) { error = delta; worstFrame = frame }; engineMixError = Math.max(engineMixError, Math.abs(rendered.getChannelData(channel)[frame] - (channel ? -nativeExpected : nativeExpected))) }
      for (let bus = 0; bus < 3; bus++) parameterError = Math.max(parameterError, Math.abs(nativeParameters.getChannelData(bus)[frame] - [1 - time / 2, time / 2, .25 + time / 4][bus]))
      positivePeak = Math.max(positivePeak, Math.abs(rendered.getChannelData(0)[frame]))
    }
    return { error, engineMixError, parameterError, worstFrame, removedPeak, positivePeak }
  }, duckedSolo)
  writeFileSync(`artifacts/volume-stage4/offline-${duckedSolo ? 'solo' : 'mix'}.json`, JSON.stringify(result, null, 2))
  expect(result.parameterError).toBeLessThan(1e-6); expect(result.engineMixError).toBeLessThan(1e-7); expect(result.error).toBeLessThan(5e-7); expect(result.removedPeak).toBe(0); expect(result.positivePeak).toBeGreaterThan(duckedSolo ? .01 : .2)
})

test('live preview schedules volume once, applies edits and rollback, and reanchors after paused seek and loop', async ({ page }, testInfo) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  const component = await (await page.request.get('/src/components/Preview.vue')).text(); const vueModule = component.match(/from\s+["']([^"']+\/vue\.js[^"']*)["']/)?.[1]; expect(vueModule).toBeTruthy()
  const audioModule = component.match(/from\s+["']([^"']*\/preview-audio\.ts[^"']*)["']/)?.[1]
  expect(audioModule).toBeTruthy()
  for (const [name, file, contentType] of [['volume-source.mp4', videoFixture, 'video/mp4'], ['volume-source.wav', audioFixture, 'audio/wav']]) await page.route(`**/${name}`, route => {
    const body = readFileSync(file); const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/); const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType, headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}) }, body: body.subarray(start, end + 1) })
  })
  await page.addInitScript(({ delayFirstVideo }) => {
    const w = window as any; w.__contexts = []; w.__mixers = []; w.__schedules = 0; w.__gains = []; w.__audioEpoch = 0; w.__scheduleEvents = []; w.__transportEvents = []
    const mediaState = () => [...document.querySelectorAll('video,audio')].map(element => ({ track: (element as HTMLElement).dataset.track, time: (element as HTMLMediaElement).currentTime, paused: (element as HTMLMediaElement).paused, seeking: (element as HTMLMediaElement).seeking }))
    w.__mediaState = mediaState
    for (const name of ['seeking', 'seeked', 'playing']) document.addEventListener(name, event => { if (event.target instanceof HTMLMediaElement) w.__transportEvents.push({ event: name, track: event.target.dataset.track, now: performance.now(), media: mediaState() }) }, true)
    if (delayFirstVideo) { const play = HTMLMediaElement.prototype.play; let delayed = false; HTMLMediaElement.prototype.play = function () { if (!delayed && this instanceof HTMLVideoElement) { delayed = true; return new Promise<void>((resolve, reject) => setTimeout(() => { play.call(this).then(resolve, reject) }, 180)) }; return play.call(this) } }
    const Context = window.AudioContext; window.AudioContext = class extends Context { constructor(options?: AudioContextOptions) { super(options); w.__contexts.push(this); const gain = this.createGain.bind(this); this.createGain = () => { const node = gain(); w.__gains.push(node); return node } } }
    const Worklet = window.AudioWorkletNode; window.AudioWorkletNode = class extends Worklet { constructor(context: BaseAudioContext, name: string, options: AudioWorkletNodeOptions) { super(context, name, options); if (name === 'screenrecord-music-mixer') { w.__mixers.push(this); const param = this.parameters.get('microphoneVolume')!; const cancel = param.cancelScheduledValues.bind(param); param.cancelScheduledValues = time => { w.__schedules++; w.__scheduleEvents.push({ epoch: w.__audioEpoch, time, now: performance.now(), media: mediaState(), stack: new Error().stack }); return cancel(time) } } } }
  }, { delayFirstVideo: true })
  await page.route('**/volume-harness', route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"><style>#fixture{display:flex;flex-direction:column;height:600px;width:900px}</style></head><body><main id="fixture"></main><script type="module">
    import { createApp, reactive, h, ref } from '${vueModule}'; import Preview from '/src/components/Preview.vue'; import { initializeTracks } from '/src/tracks.ts'; import { PreviewAudio } from '${audioModule}';
    const pause = PreviewAudio.prototype.pause; PreviewAudio.prototype.pause = function(){window.__audioEpoch++;window.__transportEvents.push({event:'audio-pause',epoch:window.__audioEpoch,now:performance.now(),media:window.__mediaState(),stack:new Error().stack});return pause.call(this)};
    const project=reactive({version:6,id:'volume-preview',name:'Synthetic volume',screenPath:'/volume-source.mp4',cameraPath:null,microphonePath:'/volume-source.wav',systemAudioPath:null,width:320,height:180,fps:30,duration:6,cursor:[],edits:{trimStart:0,trimEnd:6,aspectRatio:'16:9',background:'#202020',padding:32,cornerRadius:12,cursorSize:1,cursorSmoothing:0,showCursor:false,zooms:[],camera:{enabled:false,layout:'bubble',x:.7,y:.7,width:.2,shape:'circle',mirror:false},crop:null,systemVolume:1,microphoneVolume:.2,audioProcessing:{microphoneGain:2,systemGain:1,ducking:{enabled:false,amountDb:12},volumeKeyframes:{microphone:[{id:'level',time:0,volume:.2}]}}}});initializeTracks(project.edits);
    const state=reactive({loopRange:null});const preview=ref();const events={errors:[],times:[]};window.fixture={project,state,events,preview:()=>preview.value};
    createApp({render:()=>h(Preview,{ref:preview,project,monitor:'microphone',loopRange:state.loopRange,onTime:time=>events.times.push(time),onError:error=>events.errors.push(error)})}).mount('#fixture');
  </script></body></html>` }))
  await page.goto('/volume-harness'); await expect(page.getByText('Loading video...', { exact: true })).toBeHidden(); await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden(); await expect.poll(() => page.evaluate(() => (window as any).__mixers.length)).toBe(1)
  await page.evaluate(async () => {
    const w = window as any; const context = w.__contexts[0]; const code = "class VolumeTap extends AudioWorkletProcessor{process(inputs,outputs){const a=inputs[0]?.[0];if(a)this.port.postMessage({peak:Math.max(...a.map(Math.abs)),frame:currentFrame});for(const c of outputs[0]??[])c.fill(0);return true}}registerProcessor('volume-tap',VolumeTap)"; const url = URL.createObjectURL(new Blob([code], { type: 'text/javascript' })); await context.audioWorklet.addModule(url); URL.revokeObjectURL(url); const tap = new AudioWorkletNode(context, 'volume-tap'); w.__pcmPeak = 0; tap.port.onmessage = event => { w.__pcmPeak = event.data.peak }; w.__gains[1].connect(tap); tap.connect(context.destination); await w.fixture.preview().play()
  })
  await expect.poll(() => page.evaluate(() => (window as any).__pcmPeak)).toBeCloseTo(.048, 4)
  await expect.poll(() => page.locator('video').evaluate(element => element.currentTime)).toBeGreaterThan(.4)
  const schedulesBeforeDrift = await page.evaluate(() => (window as any).__schedules)
  await page.locator('audio[data-track="microphone recording"]').evaluate(element => { element.currentTime -= .25 })
  await expect.poll(() => page.evaluate(() => (window as any).__schedules)).toBeGreaterThan(schedulesBeforeDrift)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  const startingTime = await page.locator('video').evaluate(element => element.currentTime)
  await page.waitForTimeout(180)
  await expect.poll(() => page.locator('video').evaluate(element => element.currentTime)).toBeGreaterThan(startingTime + .08)
  await expect.poll(() => page.evaluate(() => (window as any).__pcmPeak)).toBeCloseTo(.048, 4)
  const scheduleEvidence = await page.evaluate(() => ({ schedules: (window as any).__scheduleEvents as Array<{ epoch: number; now: number }>, transport: (window as any).__transportEvents as Array<{ event: string; epoch?: number; now: number; track?: string; media: Array<{ track: string; time: number }> }>, current: (window as any).__schedules }))
  writeFileSync(path.join(directory, `schedule-diagnostic-${testInfo.repeatEachIndex}.json`), JSON.stringify(scheduleEvidence, null, 2))
  expect(new Set(scheduleEvidence.schedules.map(event => event.epoch)).size).toBe(scheduleEvidence.current)
  for (const schedule of scheduleEvidence.schedules.slice(1)) {
    const paused = scheduleEvidence.transport.find(event => event.event === 'audio-pause' && event.epoch === schedule.epoch)!
    expect(paused).toBeTruthy()
    const microphoneTime = paused.media.find(media => media.track === 'microphone recording')!.time; const videoTime = paused.media.find(media => media.track === 'screen recording')!.time
    expect(Math.abs(microphoneTime - videoTime - 239 / 48000)).toBeGreaterThan(.119)
    expect(scheduleEvidence.transport.some(event => event.event === 'seeked' && event.track === 'microphone recording' && event.now >= paused.now && event.now <= schedule.now)).toBe(true)
  }
  await page.evaluate(() => { (window as any).fixture.project.edits.audioProcessing.volumeKeyframes.microphone[0].volume = .8 }); await expect.poll(() => page.evaluate(() => (window as any).__pcmPeak)).toBeCloseTo(.192, 4)
  await page.evaluate(() => { (window as any).fixture.project.edits.audioProcessing.volumeKeyframes.microphone[0].volume = .2 }); await expect.poll(() => page.evaluate(() => (window as any).__pcmPeak)).toBeCloseTo(.048, 4)
  await page.evaluate(() => { const f = (window as any).fixture; f.preview().pause(); f.project.edits.audioProcessing.volumeKeyframes.microphone = [{ id: 'a', time: 0, volume: 0 }, { id: 'b', time: 2, volume: 1 }]; f.preview().seek(.5) }); await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await page.evaluate(() => { const f = (window as any).fixture; f.state.loopRange = { start: .5, end: .8 }; f.events.times.length = 0; return f.preview().play() })
  await expect.poll(() => page.evaluate(() => (window as any).fixture.events.times.filter((time: number) => Math.abs(time - .5) < 1e-5).length)).toBeGreaterThanOrEqual(3)
  const param = await page.evaluate(() => { const w = window as any; return { current: w.__mixers[0].parameters.get('microphoneVolume').value, time: document.querySelector<HTMLVideoElement>('video')!.currentTime } }); expect(param.current).toBeGreaterThan(.24); expect(param.current).toBeLessThan(.42)
  await page.evaluate(() => (window as any).fixture.preview().pause()); await expect.poll(() => page.evaluate(() => (window as any).__contexts[0].state)).toBe('suspended'); await page.evaluate(() => (window as any).fixture.preview().release()); await expect.poll(() => page.evaluate(() => (window as any).__contexts[0].state)).toBe('closed')
  expect(errors).toEqual([]); expect(await page.evaluate(() => (window as any).fixture.events.errors)).toEqual([])
})
