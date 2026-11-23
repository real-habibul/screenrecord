import { expect, test } from '@playwright/test'
import { createReadStream, statSync } from 'node:fs'
import { createServer } from 'node:http'
import path from 'node:path'

const recordingDirectory = process.env.SCREENRECORD_MEDIA_TEST_DIR

test('captured screen and float audio decode in the browser media engine', async ({ page }) => {
  test.skip(!recordingDirectory, 'Set SCREENRECORD_MEDIA_TEST_DIR to an actual captured project folder.')
  const server = createServer((request, response) => {
    const name = request.url === '/screen.mp4' ? 'screen.mp4' : request.url === '/system.wav' ? 'system.wav' : null
    if (!name) { response.writeHead(404).end(); return }
    const file = path.join(recordingDirectory!, name); const size = statSync(file).size
    const range = request.headers.range?.match(/bytes=(\d+)-(\d*)/)
    const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), size - 1) : size - 1
    response.writeHead(range ? 206 : 200, { 'Content-Type': name.endsWith('.wav') ? 'audio/wav' : 'video/mp4', 'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes', 'Access-Control-Allow-Origin': '*', ...(range ? { 'Content-Range': `bytes ${start}-${end}/${size}` } : {}) })
    createReadStream(file, { start, end }).pipe(response)
  })
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address() as { port: number }
  try {
  await page.goto('/')
  await page.setContent(`<button id="play">Play</button><video id="screen" muted src="http://127.0.0.1:${address.port}/screen.mp4"></video><audio id="audio" crossorigin="anonymous" src="http://127.0.0.1:${address.port}/system.wav"></audio>`, { waitUntil: 'domcontentloaded' })
  await page.evaluate(() => {
    document.getElementById('play')!.onclick = async () => {
      const audio = document.getElementById('audio') as HTMLAudioElement
      try {
        const context = new AudioContext({ sampleRate: 48_000 })
        await context.audioWorklet.addModule('/audio-limiter.js')
        const source = context.createMediaElementSource(audio)
        const limiter = new AudioWorkletNode(context, 'screenrecord-limiter', { outputChannelCount: [2], channelCount: 2, channelCountMode: 'explicit' })
        const silent = context.createGain(); silent.gain.value = 0
        source.connect(limiter); limiter.connect(silent); silent.connect(context.destination)
        await context.resume(); audio.dataset.processor = context.state
        ;(window as any).__mediaContext = context
      } catch (error) { audio.dataset.processor = String(error); return }
      for (const media of document.querySelectorAll<HTMLMediaElement>('video,audio')) {
        try { await media.play(); media.dataset.result = 'playing' } catch (error) { media.dataset.result = String(error) }
      }
    }
  })
  await page.locator('#play').click()
  await expect(page.locator('#screen')).toHaveAttribute('data-result', 'playing')
  await expect(page.locator('#audio')).toHaveAttribute('data-result', 'playing')
  await expect(page.locator('#audio')).toHaveAttribute('data-processor', 'running')
  await expect.poll(() => page.locator('#screen').evaluate(el => (el as HTMLVideoElement).currentTime)).toBeGreaterThan(.1)
  await expect.poll(() => page.locator('#audio').evaluate(el => (el as HTMLAudioElement).currentTime)).toBeGreaterThan(.1)
  await page.evaluate(async () => { document.querySelectorAll<HTMLMediaElement>('video,audio').forEach(media => media.pause()); await (window as any).__mediaContext.close() })
  } finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())) }
})
