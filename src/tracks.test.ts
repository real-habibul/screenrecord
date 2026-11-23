import { describe, expect, it } from 'vitest'
import { EditHistory } from './history'
import { PreviewImageCache } from './image-cache'
import { dragSourceTrim, dragTrack, dragZoom, initializeTracks, musicGain, redactionsFit, sourceRectangle, trackLanes, updateInterval, updateMusic } from './tracks'
import type { EditSettings, MusicClip, Redaction } from './types'

const music = (): MusicClip => ({ id: 'song', name: 'Music', path: 'music.wav', sourceDuration: 30, start: 5, sourceStart: 3, duration: 10, volume: .5, fadeIn: 2, fadeOut: 3 })
const tracks = (): EditSettings => ({ music: [music()], subtitles: [{ id: 'before', start: 1, end: 2, text: 'First' }, { id: 'cue', start: 3, end: 4, text: 'Second' }, { id: 'after', start: 6, end: 7, text: 'Third' }], redactions: [] } as unknown as EditSettings)
const region = (id: string, start: number, end: number): Redaction => ({ id, start, end, x: .1, y: .1, width: .2, height: .2, mode: 'solid', strength: 16 })

describe('editor tracks', () => {
  it('upgrades legacy edit defaults without changing existing tracks', () => {
    const e = {} as EditSettings; initializeTracks(e)
    expect(e).toMatchObject({ music: [], subtitles: [], redactions: [], subtitleStyle: { enabled: true, fontSize: 42 } })
    e.music.push(music()); initializeTracks(e); expect(e.music).toHaveLength(1)
  })
  it('uses source-time fades, with silence outside the half-open music interval', () => {
    const clip = music()
    expect([4, 5, 6, 7, 12, 13.5, 15].map(time => musicGain(clip, time))).toEqual([0, 0, .25, .5, .5, .25, 0])
    clip.fadeIn = 0; clip.fadeOut = 0
    expect(musicGain(clip, 5)).toBe(.5); expect(musicGain(clip, 15)).toBe(0)
  })
  it('trims music source and caps fades without extending beyond source or project', () => {
    const e = tracks(); const original = music()
    dragTrack(e, { kind: 'music', id: 'song' }, original, 'start', -10, 20)
    expect(e.music[0]).toMatchObject({ start: 2, sourceStart: 0, duration: 13 })
    dragTrack(e, { kind: 'music', id: 'song' }, original, 'end', 100, 20)
    expect(e.music[0].duration).toBe(15)
    updateMusic(e.music[0], 'duration', 1, 20)
    expect(e.music[0].fadeIn + e.music[0].fadeOut).toBeLessThanOrEqual(1)
  })
  it('moves subtitles up to neighbors and retains a valid 10ms generated cue', () => {
    const e = tracks(); const original = { ...e.subtitles[1] }
    dragTrack(e, { kind: 'subtitles', id: 'cue' }, original, 'move', 10, 20)
    expect(e.subtitles[1]).toMatchObject({ start: 5, end: 6 })
    const short = { id: 'short', start: 11.99, end: 12, text: 'Ya' }
    updateInterval(short, 'start', 11.99, 12, [], .01)
    expect(short.start).toBe(11.99); expect(short.end).toBe(12)
  })
  it('checks simultaneous redactions with end-before-start boundaries', () => {
    const regions = Array.from({ length: 16 }, (_, i) => region(String(i), 0, 1))
    expect(redactionsFit([...regions, region('later', 1, 2)])).toBe(true)
    expect(redactionsFit([...regions, region('overlap', .5, 2)])).toBe(false)
    const e = tracks(); e.redactions = [...regions, region('later', 1, 2)]
    expect(dragTrack(e, { kind: 'redactions', id: 'later' }, { ...e.redactions[16] }, 'move', -.5, 12)).toBe(false)
    expect(e.redactions[16].start).toBe(1)
  })
  it('covers fractional source pixels and packs non-overlapping clips onto one lane', () => {
    expect(sourceRectangle({ x: .101, y: .203, width: .202, height: .201 }, 100, 100)).toEqual({ x: 10, y: 20, width: 21, height: 21 })
    expect(trackLanes([{ start: 0, end: 1 }, { start: 1, end: 2 }, { start: .5, end: 1.5 }]).map(item => item.lane)).toEqual([0, 1, 0])
  })
  it('trims the primary recording without shortening below100ms or crossing source bounds', () => {
    const e = tracks(); e.trimStart = 2; e.trimEnd = 10
    dragSourceTrim(e, { start: 2, end: 10 }, 'start', 20, 12)
    expect(e.trimStart).toBe(9.9)
    dragSourceTrim(e, { start: 2, end: 10 }, 'end', 20, 12)
    expect(e.trimEnd).toBe(12)
  })
  it('moves and trims zooms up to adjacent segments without overwriting focus or scale', () => {
    const e = tracks(); e.zooms = [{ id: 'a', start: 1, end: 2, scale: 2, x: .2, y: .3, followCursor: true }, { id: 'b', start: 3, end: 5, scale: 1.7, x: .4, y: .6, followCursor: false }, { id: 'c', start: 7, end: 9, scale: 2, x: .5, y: .5, followCursor: true }]
    const original = { ...e.zooms[1] }
    dragZoom(e, original, 'move', 30, 12)
    expect(e.zooms[1]).toMatchObject({ start: 5, end: 7, scale: 1.7, x: .4, y: .6, followCursor: false })
    dragZoom(e, original, 'start', -20, 12)
    expect(e.zooms[1].start).toBe(2)
  })
})

describe('edit history', () => {
  it('coalesces an edit gesture, allows redo and discards redo after a new edit', () => {
    const history = new EditHistory(); history.reset('initial')
    history.observe('drag1'); history.observe('drag2'); history.observe('drag3'); history.commit()
    expect(history.undo()).toBe('initial'); expect(history.redo()).toBe('drag3')
    expect(history.undo()).toBe('initial'); history.observe('different'); history.commit()
    expect(history.canRedo).toBe(false); expect(history.undo()).toBe('initial')
  })
  it('bounds stored UTF-16 snapshots by bytes and count while keeping recent undo/redo', () => {
    const history = new EditHistory(100, 240); history.reset('0'.repeat(40))
    for (let i = 1; i <= 10; i++) { history.observe(String(i).padStart(40, 'x')); history.commit() }
    expect(history.undo()).toBe('9'.padStart(40, 'x'))
    expect(history.redo()).toBe('10'.padStart(40, 'x'))
    let count = 0; while (history.undo() !== null) count++
    expect(count).toBeLessThanOrEqual(3)
    const bounded = new EditHistory(1); bounded.reset('a'); bounded.observe('b'); bounded.commit(); bounded.observe('c'); bounded.commit()
    expect(bounded.undo()).toBe('b'); expect(bounded.undo()).toBeNull()
  })
})

describe('subtitle image memory', () => {
  it('protects both active word and plain fallback, and favors the active image at the byte limit', () => {
    const image = () => ({ naturalWidth: 100, naturalHeight: 100, src: 'loaded.png' })
    const cache = new PreviewImageCache(80_000, 8)
    const base = image(); const word = image(); const future = image()
    cache.put('base', base, ['word', 'base']); cache.put('word', word, ['word', 'base'])
    expect(cache.put('future', future, ['word', 'base'])).toBe(false)
    expect(cache.has('base')).toBe(true); expect(cache.has('word')).toBe(true); expect(future.src).toBe('')
    const smaller = new PreviewImageCache(40_000, 8)
    smaller.put('base', image(), ['word', 'base']); smaller.put('word', image(), ['word', 'base'])
    expect(smaller.has('base')).toBe(false); expect(smaller.has('word')).toBe(true)
  })
  it('keeps the active raster and drops prefetch when decoded pixels exceed the budget', () => {
    const cache = new PreviewImageCache(64 * 1024 * 1024)
    const image = () => ({ naturalWidth: 8192, naturalHeight: 1080, src: 'loaded.png' })
    const active = image(); const next = image()
    expect(cache.put('active', active, 'active')).toBe(true)
    expect(cache.put('next', next, 'active')).toBe(false)
    expect(active.src).toBe('loaded.png'); expect(next.src).toBe('')
    expect(cache.bytes).toBeLessThanOrEqual(64 * 1024 * 1024)
    const sought = image(); expect(cache.put('next', sought, 'next')).toBe(true)
    expect(active.src).toBe(''); cache.clear(); expect(sought.src).toBe('')
  })
})
