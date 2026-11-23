import type { Route } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import path from 'node:path'

export const editorVideo = path.resolve('artifacts/ui/source.mp4')

export function mediaEngine() {
  const bundled = path.resolve('src-tauri/resources/ffmpeg.exe')
  return process.env.SCREENRECORD_FFMPEG ?? (existsSync(bundled) ? bundled : 'ffmpeg')
}

export function publishFixture(file: string, generate: (temporaryFile: string) => void) {
  if (existsSync(file)) return
  const directory = path.dirname(file)
  mkdirSync(directory, { recursive: true })
  const temporaryDirectory = mkdtempSync(path.join(directory, '.fixture-'))
  const temporaryFile = path.join(temporaryDirectory, path.basename(file))
  try {
    generate(temporaryFile)
    try {
      linkSync(temporaryFile, file)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
    }
  } finally {
    rmSync(temporaryDirectory, { recursive: true, force: true })
  }
}

export function generateMedia(file: string, engine: string, args: string[]) {
  publishFixture(file, output => {
    execFileSync(engine, ['-hide_banner', '-loglevel', 'error', ...args, output], { windowsHide: true })
  })
}

export function prepareEditorVideo(engine: string, encoder: string[]) {
  generateMedia(editorVideo, engine, [
    '-f', 'lavfi', '-i', 'testsrc2=size=960x540:rate=30', '-t', '12', ...encoder,
    '-pix_fmt', 'yuv420p', '-movflags', '+faststart',
  ])
}

export function serveMedia(route: Route, file: string, contentType: string, allowedOrigin?: string) {
  const body = readFileSync(file)
  const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range ?? '')
  const start = range ? Number(range[1]) : 0
  const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
  const headers: Record<string, string> = { 'Accept-Ranges': 'bytes' }
  if (allowedOrigin) headers['Access-Control-Allow-Origin'] = allowedOrigin
  if (range && start > end) {
    headers['Content-Range'] = `bytes */${body.length}`
    return route.fulfill({ status: 416, headers })
  }
  if (range) headers['Content-Range'] = `bytes ${start}-${end}/${body.length}`
  headers['Content-Length'] = String(end - start + 1)
  return route.fulfill({ status: range ? 206 : 200, contentType, headers, body: body.subarray(start, end + 1) })
}
