import type { CursorSample } from './types'

export type ClickButton = 'left' | 'right' | 'generic'
export interface CursorClick { time: number; x: number; y: number; button: ClickButton; end: number }
export type ClickIndex = Record<ClickButton, CursorClick[]>
const channels: ClickButton[] = ['left', 'right', 'generic']
export const CLICK_DURATION = .4

export function cursorClicks(samples: readonly CursorSample[]): CursorClick[] {
  const clicks: CursorClick[] = []; let previous = 0; let nextHidden = 0
  for (let index = 0; index < samples.length; index++) {
    const sample = samples[index]; const typed = (sample.buttons ?? 0) & 7
    const held = typed || (sample.pressed ? 4 : 0)
    const rising = held & ~previous
    previous = held
    if (!rising || !sample.visible || index === 0 && typed) continue
    if (nextHidden <= index) { nextHidden = index; while (nextHidden < samples.length && samples[nextHidden].visible) nextHidden++ }
    const end = Math.min(sample.time + CLICK_DURATION, samples[nextHidden]?.time ?? Infinity)
    if (end <= sample.time) continue
    for (const [bit, button] of [[1, 'left'], [2, 'right'], [4, 'generic']] as const) if (rising & bit) clicks.push({ time: sample.time, x: sample.x, y: sample.y, button, end })
  }
  return clicks
}

export function indexClicks(clicks: readonly CursorClick[]): ClickIndex {
  const index: ClickIndex = { left: [], right: [], generic: [] }
  for (const click of clicks) index[click.button].push(click)
  return index
}

export function clicksAt(index: ClickIndex, time: number, ranges: readonly { start: number; end: number }[]): CursorClick[] {
  if (!index.left.length && !index.right.length && !index.generic.length) return []
  let start = 0; let end = ranges.length
  while (start < end) { const mid = Math.floor((start + end) / 2); if (ranges[mid].start <= time) start = mid + 1; else end = mid }
  const range = ranges[start - 1]
  if (!range || time >= range.end) return []
  const active: CursorClick[] = []
  for (const button of channels) {
    const items = index[button]; let lo = 0; let hi = items.length
    while (lo < hi) { const mid = Math.floor((lo + hi) / 2); if (items[mid].time <= time) lo = mid + 1; else hi = mid }
    const click = items[lo - 1]
    if (click && click.time >= range.start && time < click.end && time - click.time < CLICK_DURATION) active.push(click)
  }
  return active
}

export function clickStyle(click: CursorClick, time: number, scale: number) {
  const phase = Math.max(0, Math.min(1, (time - click.time) / CLICK_DURATION))
  return { radius: (12 + 16 * phase) * scale, width: 2.5 * scale, backingWidth: 4.5 * scale, opacity: .95 * (1 - phase), color: click.button === 'left' ? '#F4B64A' : click.button === 'right' ? '#66D9EF' : '#EEEEEE', double: click.button === 'right' }
}
