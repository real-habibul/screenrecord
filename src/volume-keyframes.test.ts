import { expect, it } from 'vitest'
import { insertVolumeKey, moveVolumeKey, projectVolumeKeys, volumeAt } from './volume-keyframes'
import { keptClips } from './video-edits'

const keys = [{ id: 'a', time: 2, volume: 0 }, { id: 'inside-cut', time: 4, volume: 1 }, { id: 'b', time: 6, volume: .5 }]
it('holds endpoints and evaluates automation in original recording seconds', () => {
  expect([0, 2, 3, 4, 5, 6, 7].map(time => volumeAt(keys, time))).toEqual([0, 0, .5, 1, .75, .5, .5])
  expect(volumeAt([], 2)).toBe(1)
  const clips = keptClips({ trimStart: 0, trimEnd: 8, videoClips: [{ id: 'left', start: 0, end: 3 }, { id: 'right', start: 5, end: 8 }] })
  const { paths, visible } = projectVolumeKeys(keys, clips, 6)
  expect(paths).toHaveLength(2)
  expect(paths[0].endsWith('50,50')).toBe(true)
  expect(paths[1].startsWith('50,25')).toBe(true)
  expect(visible.map(({ key, output }) => [key.id, output])).toEqual([['a', 2], ['b', 4]])
})
it('protects point spacing, keeps the selected ID and leaves the input intact', () => {
  const replaced = insertVolumeKey(keys, 2.0002, .15, 8)
  expect(replaced.selected).toBe('a')
  expect(replaced.keys[0]).toEqual({ id: 'a', time: 2, volume: .15 })
  const added = insertVolumeKey(keys, 1, .3, 8)
  expect(added.keys.map(key => key.time)).toEqual([1, 2, 4, 6])
  expect(moveVolumeKey(keys, 'inside-cut', 8, -1, 8)[1]).toEqual({ id: 'inside-cut', time: 5.999, volume: 0 })
  expect(keys[1].volume).toBe(1)
})
