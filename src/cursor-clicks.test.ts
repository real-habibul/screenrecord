import { describe, expect, it } from 'vitest'
import fixture from '../testdata/cursor-clicks.json'
import { clickStyle, clicksAt, cursorClicks, indexClicks, type CursorClick } from './cursor-clicks'
import { playbackRanges } from './video-edits'
import type { CursorSample } from './types'

const sample = (time: number, buttons = 0, visible = true): CursorSample => ({ time, x: .25, y: .75, buttons, pressed: buttons !== 0, visible })
const ranges = [{ start: 0, end: 20 }]

describe('recorded click pulses', () => {
  it('matches the shared Rust fixture including equal-time resume markers and raw coordinates', () => {
    const clicks = cursorClicks(fixture.samples)
    expect(clicks).toHaveLength(fixture.clicks.length)
    clicks.forEach((click, index) => { expect({ ...click, end: undefined }).toEqual({ ...fixture.clicks[index], end: undefined }); expect(click.end).toBeCloseTo(fixture.clicks[index].end, 9) })
  })
  it('establishes initial typed holds and preserves a legacy neutral pulse at time zero', () => {
    expect(cursorClicks([sample(0, 3)])).toEqual([])
    const legacy = { ...sample(0), pressed: true }; delete legacy.buttons
    const clicks = cursorClicks([legacy])
    expect(clicks).toEqual([{ time: 0, x: .25, y: .75, button: 'generic', end: .4 }])
    expect(clickStyle(clicks[0], 0, 1)).toMatchObject({ radius: 12, opacity: .95, color: '#EEEEEE' })
  })
  it('does not repeat stationary holds, create reentry clicks, or infer a release across long gaps', () => {
    const clicks = cursorClicks([sample(0, 0, false), sample(.01, 1), sample(.02, 1), sample(.1, 1, false), sample(.11, 1), sample(5, 1), sample(5.1, 3), sample(5.2), sample(5.3, 4), { ...sample(5.4), pressed: true }])
    expect(clicks.map(click => [click.time, click.button, click.end])).toEqual([[.01, 'left', .1], [5.1, 'right', 5.5], [5.3, 'generic', 5.7]])
  })
  it('shows only the newest pulse per channel and never resurrects one after invisibility', () => {
    const clicks = indexClicks(cursorClicks([sample(0), sample(.1, 3), sample(.11), { ...sample(.2, 1), x: .8 }, sample(.25, 0, false), sample(.3)]))
    expect(clicksAt(clicks, .21, ranges).map(click => [click.time, click.button, click.x])).toEqual([[.2, 'left', .8], [.1, 'right', .25]])
    expect(clicksAt(clicks, .25, ranges)).toEqual([]); expect(clicksAt(clicks, .31, ranges)).toEqual([])
  })
  it('does not carry clicks across removed trim/cut times but preserves truly adjacent splits', () => {
    const index = indexClicks(cursorClicks([sample(0), sample(.9, 1), sample(.95), sample(1.05, 2)]))
    const cut = playbackRanges({ trimStart: 0, trimEnd: 2, videoClips: [{ id: 'a', start: 0, end: 1 }, { id: 'b', start: 1.1, end: 2 }] })
    expect(clicksAt(index, 1.15, cut)).toEqual([])
    expect(clicksAt(index, 1, [{ start: 1, end: 2 }])).toEqual([])
    const adjacent = playbackRanges({ trimStart: 0, trimEnd: 2, videoClips: [{ id: 'a', start: 0, end: 1 }, { id: 'b', start: 1, end: 2 }] })
    expect(clicksAt(index, 1, adjacent).map(click => click.button)).toEqual(['left'])
    expect(clicksAt(index, 1.46, adjacent)).toEqual([])
  })
  it('expands and fades at source time with exact button styles and cursor-size scaling', () => {
    const click: CursorClick = { time: 2, end: 2.4, x: .5, y: .5, button: 'right' }
    const style = clickStyle(click, 2.2, 2)
    expect(style.radius).toBeCloseTo(40); expect(style.opacity).toBeCloseTo(.475); expect(style).toMatchObject({ width: 5, backingWidth: 9, color: '#66D9EF', double: true })
    expect(clicksAt(indexClicks([click]), 2.4, ranges)).toEqual([])
  })
  it('queries large recordings by binary search rather than scanning past clicks every frame', () => {
    const entries = Array.from({ length: 100_000 }, (_, index): CursorClick => ({ time: index / 10, end: index / 10 + .4, x: .5, y: .5, button: 'left' }))
    let reads = 0; const indexed = new Proxy(entries, { get(target, key, receiver) { if (typeof key === 'string' && /^\d+$/.test(key)) reads++; return Reflect.get(target, key, receiver) } })
    expect(clicksAt({ left: indexed, right: [], generic: [] }, 9999.95, [{ start: 0, end: 10_000 }])).toHaveLength(1)
    expect(reads).toBeLessThan(25)
  })
  it('skips interval work for an empty track and finds a kept interval without scanning a long cut list', () => {
    const empty = indexClicks([])
    const unavailable = new Proxy([] as { start: number; end: number }[], { get() { throw new Error('Empty clicks must not read ranges') } })
    expect(clicksAt(empty, 1, unavailable)).toEqual([])
    let reads = 0; const intervals = new Proxy(Array.from({ length: 512 }, (_, i) => ({ start: i, end: i + .5 })), { get(target, key, receiver) { if (typeof key === 'string' && /^\d+$/.test(key)) reads++; return Reflect.get(target, key, receiver) } })
    const click: CursorClick = { time: 511.1, end: 511.5, button: 'left', x: .5, y: .5 }
    expect(clicksAt(indexClicks([click]), 511.2, intervals)).toEqual([click]); expect(reads).toBeLessThan(13)
    expect(clicksAt(indexClicks([click]), 511.5, intervals)).toEqual([])
  })
})
