import { describe, expect, it } from 'vitest'
import reference from '../testdata/reframe-reference.json'
import { appearanceSettings, builtInPresets } from './appearance'
import { previewGeometry } from './editing'
import type { Project } from './types'

describe('shared Fit, Fill and manual framing geometry', () => {
  for (const value of reference.cases) it(value.id, () => {
    const appearance = appearanceSettings(builtInPresets[1].settings)
    const project = { ...value.source, edits: { ...appearance, ...value.edits, camera: { ...appearance.camera, ...value.edits.camera } } } as unknown as Project
    const geometry = previewGeometry(project, value.time, value.cursor)
    expect({
      canvas: { width: geometry.width, height: geometry.height },
      frame: { x: geometry.fx, y: geometry.fy, width: geometry.fw, height: geometry.fh },
      viewport: { x: geometry.sx, y: geometry.sy, width: geometry.sw, height: geometry.sh },
      screenVisible: geometry.screenVisible,
      cameraPane: geometry.cameraPane,
    }).toEqual(value.expected)
  })
})
