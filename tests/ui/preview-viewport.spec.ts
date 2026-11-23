import { expect, test, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import type { Project } from '../../src/types'

const fixture = path.resolve('artifacts/ui/source.mp4')
const audioFixture = path.resolve('artifacts/ui/preview-release-audio.wav')
const sparseFixture = path.resolve('artifacts/ui/preview-sparse-vfr.mp4')
const clicksFixture = path.resolve('artifacts/ui/preview-clicks.mp4')
test.beforeAll(() => {
  mkdirSync(path.dirname(fixture), { recursive: true })
  const bundled = path.resolve('src-tauri/resources/ffmpeg.exe')
  const engine = process.env.SCREENRECORD_FFMPEG ?? (existsSync(bundled) ? bundled : 'ffmpeg')
  const encoder = process.platform === 'win32' ? ['-c:v', 'h264_mf', '-hw_encoding', '0', '-b:v', '4000k'] : ['-c:v', 'libx264', '-preset', 'ultrafast']
  if (!existsSync(fixture)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=960x540:rate=30', '-t', '12', ...encoder, '-pix_fmt', 'yuv420p', '-movflags', '+faststart', fixture], { windowsHide: true })
  if (!existsSync(audioFixture)) execFileSync(engine,['-hide_banner','-loglevel','error','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','12','-ac','2','-c:a','pcm_f32le',audioFixture],{windowsHide:true})
  if (!existsSync(sparseFixture)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=blue:size=320x180:rate=10:duration=4', '-vf', "select='eq(n,0)+eq(n,1)+eq(n,10)+eq(n,11)+eq(n,30)'", '-fps_mode', 'vfr', ...encoder, '-pix_fmt', 'yuv420p', '-movflags', '+faststart', sparseFixture], { windowsHide: true })
  if (!existsSync(clicksFixture)) execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=black:size=960x540:rate=30:duration=12', ...encoder, '-pix_fmt', 'yuv420p', '-movflags', '+faststart', clicksFixture], { windowsHide: true })
})

type RecordedSources = Pick<Project, 'cameraPath' | 'microphonePath' | 'systemAudioPath' | 'cameraOffsetSeconds'>

async function harness(page: Page, sources: Partial<RecordedSources> = {}) {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const componentModule = await (await page.request.get('/src/components/Preview.vue')).text()
  const vueModule = componentModule.match(/from\s+["']([^"']+\/vue\.js[^"']*)["']/)?.[1]
  expect(vueModule).toBeTruthy()
  await page.route('**/viewport-audio.wav', route => {
    const body = readFileSync(audioFixture); const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/)
    const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType: 'audio/wav', headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}) }, body: body.subarray(start, end + 1) })
  })
  await page.route('**/viewport-source.mp4', route => {
    const body = readFileSync(fixture)
    const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/)
    const start = range ? Number(range[1]) : 0
    const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}) }, body: body.subarray(start, end + 1) })
  })
  await page.route('**/viewport-harness', route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><link rel="stylesheet" href="/src/style.css"><style>body{min-width:0}#fixture{display:flex;flex-direction:column;height:600px;width:900px;max-width:100vw}</style></head><body><main id="fixture"></main><script type="module">
    import { createApp, reactive, h, ref } from '${vueModule}';
    import Preview from '/src/components/Preview.vue';
    import { previewGeometry } from '/src/editing.ts';
    import { EditHistory } from '/src/history.ts';
    const project = reactive({ version:3,id:'viewport-test',name:'Viewport test',createdAt:'2026-11-11T00:00:00Z',directory:'',screenPath:'/viewport-source.mp4',cameraPath:null,microphonePath:null,systemAudioPath:null,cameraOffsetSeconds:0,width:960,height:540,fps:30,duration:12,cursor:[],status:'ready',edits:{trimStart:1,trimEnd:12,aspectRatio:'16:9',background:'#202020',padding:32,cornerRadius:12,cursorSize:1,cursorSmoothing:0,showCursor:false,zooms:[],camera:{enabled:false,x:.7,y:.7,width:.2,shape:'circle',mirror:false},crop:null,systemVolume:1,microphoneVolume:1,music:[],subtitles:[],subtitleStyle:{enabled:false,fontSize:44,color:'#ffffff',background:'#000000',position:'bottom'},redactions:[]}});
    Object.assign(project, ${JSON.stringify(sources)});
    const state = reactive({ drawing:false,selectedRegion:null,selectedCamera:false,disabled:false,originalFrame:false });
    const preview = ref(); const history = new EditHistory();
    const snapshot = () => JSON.stringify(project.edits); history.reset(snapshot());
    const events = {regions:[],times:[],errors:[],keys:[],begins:0,ends:[],selections:[]};
    window.addEventListener('keydown',event => events.keys.push(event.key));
    window.fixture = {project,state,events,preview:()=>preview.value,resetHistory:()=>history.reset(snapshot()),undo:()=>{const value=history.undo();if(value)project.edits=JSON.parse(value);return value!==null},geometry:()=>previewGeometry(project,document.querySelector('video').currentTime,null)};
    createApp({render:()=>h(Preview,{ref:preview,project,drawing:state.drawing,disabled:state.disabled,originalFrame:state.originalFrame,selectedRegion:state.selectedRegion,selectedCamera:state.selectedCamera,onSelectRegion:id=>{state.selectedRegion=id;state.selectedCamera=false;events.selections.push(id)},onSelectCamera:()=>{state.selectedCamera=true;state.selectedRegion=null;events.selections.push('camera')},onBeginEdit:()=>{history.commit();events.begins++},onEndEdit:commit=>{history.observe(snapshot());history.commit();events.ends.push(commit)},onUpdateRegion:value=>{Object.assign(project.edits.redactions.find(item=>item.id===value.id),value);history.observe(snapshot())},onUpdateCamera:value=>{Object.assign(project.edits.camera,value);history.observe(snapshot())},onRegion:region=>events.regions.push(region),onTime:time=>events.times.push(time),onError:error=>events.errors.push(error)})}).mount('#fixture');
  </script></body></html>` }))
  await page.goto('/viewport-harness')
  await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
  await expect.poll(() => page.locator('canvas').evaluate(el => (el as HTMLCanvasElement).width)).toBeGreaterThan(100)
  expect(errors).toEqual([])
  return errors
}

test.use({ deviceScaleFactor: 2 })

const sourceCombinations = [
  { name: 'screen only', mic: false, system: false, camera: false },
  { name: 'microphone only', mic: true, system: false, camera: false },
  { name: 'computer audio only', mic: false, system: true, camera: false },
  { name: 'webcam only', mic: false, system: false, camera: true },
  { name: 'both audio sources and webcam', mic: true, system: true, camera: true },
]

for (const tracks of sourceCombinations) {
  test(`playback survives edits and seeks with ${tracks.name}`, async ({ page }) => {
    const errors = await harness(page, {
      microphonePath: tracks.mic ? '/viewport-audio.wav' : null,
      systemAudioPath: tracks.system ? '/viewport-audio.wav' : null,
      cameraPath: tracks.camera ? '/viewport-source.mp4' : null,
      cameraOffsetSeconds: tracks.camera ? .25 : 0,
    })
    const screen = page.locator('video[data-track="screen recording"]')
    const overlay = page.locator('.preview-loading')
    await expect(overlay).toBeHidden()
    await page.evaluate(async () => {
      const f = (window as any).fixture
      await f.preview().play()
      const screen = document.querySelector<HTMLVideoElement>('video')!
      screen.dataset.pauses = '0'
      screen.addEventListener('pause', () => { screen.dataset.pauses = String(Number(screen.dataset.pauses) + 1) })
      f.project.edits.microphoneVolume = 0
      f.project.edits.systemVolume = 0
      f.project.edits.camera.x = .25
    })
    const start = await screen.evaluate(el => el.currentTime)
    await expect.poll(() => screen.evaluate(el => el.currentTime)).toBeGreaterThan(start + .5)
    expect(await screen.getAttribute('data-pauses')).toBe('0')
    await expect(overlay).toBeHidden()

    await page.evaluate(() => (window as any).fixture.preview().seek(6))
    await expect.poll(() => screen.evaluate(el => el.currentTime)).toBeGreaterThan(6.2)
    await page.evaluate(tracks => {
      const edits = (window as any).fixture.project.edits
      edits.microphoneVolume = .4
      edits.systemVolume = .8
      edits.camera.enabled = tracks.camera
    }, tracks)
    const expectedTracks = Number(tracks.mic) + Number(tracks.system) + Number(tracks.camera)
    const auxiliary = page.locator('audio, video[data-track="webcam recording"]')
    await expect(auxiliary).toHaveCount(expectedTracks)
    await expect.poll(() => page.evaluate(() => {
      const screen = document.querySelector<HTMLVideoElement>('video')!
      return [...document.querySelectorAll<HTMLMediaElement>('audio, video[data-track="webcam recording"]')].every(el => {
        const offset = el instanceof HTMLVideoElement ? .25 : 0
        return !el.paused && Math.abs(el.currentTime - offset - screen.currentTime) < .12
      }) && !screen.paused
    })).toBe(true)
    await page.evaluate(() => {
      const f = (window as any).fixture
      f.preview().pause()
      f.preview().seek(3)
      f.project.edits.camera.y = .4
    })
    await expect(overlay).toBeHidden()
    await expect.poll(() => screen.evaluate(el => el.currentTime)).toBeCloseTo(3, 4)
    expect(await page.locator('video, audio').evaluateAll(elements => elements.every(el => (el as HTMLMediaElement).paused))).toBe(true)
    expect(await page.evaluate(() => (window as any).fixture.events.errors)).toEqual([])
    expect(errors).toEqual([])
  })
}

test('returning to a paused preview paints edits made while the document was hidden', async ({ page }) => {
  await harness(page)
  const overlay = page.locator('.preview-loading')
  await expect(overlay).toBeHidden()
  const pixel = () => page.locator('canvas.preview-canvas').evaluate(canvas =>
    [...canvas.getContext('2d')!.getImageData(0, 0, 1, 1).data],
  )
  await expect.poll(pixel).toEqual([32, 32, 32, 255])
  const position = await page.locator('video').evaluate(el => el.currentTime)
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' })
    document.dispatchEvent(new Event('visibilitychange'))
    const edits = (window as any).fixture.project.edits
    edits.background = '#544332'
    edits.microphoneVolume = 0
    edits.camera.x = .4
  })
  await page.evaluate(() => {
    Reflect.deleteProperty(document, 'visibilityState')
    document.dispatchEvent(new Event('visibilitychange'))
  })
  await expect.poll(pixel).toEqual([84, 67, 50, 255])
  await expect.poll(() => page.locator('canvas.preview-canvas').evaluate(canvas => {
    const data = canvas.getContext('2d')!.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data
    return data[0] === 84 && data[1] === 67 && data[2] === 50
  })).toBe(false)
  await expect(page.locator('video')).toHaveJSProperty('paused', true)
  await expect(page.locator('video')).toHaveJSProperty('currentTime', position)
  await expect(overlay).toBeHidden()
})

test('Fit, 100%, and 200% retain canonical scale and support keyboard pan without editing the project', async ({ page }) => {
  const errors = await harness(page)
  const canvas = page.getByLabel('Edited video preview')
  const viewport = page.getByRole('region', { name: 'Video preview viewport' })
  const scale = page.getByLabel('Preview scale', { exact: true })
  const edits = await page.evaluate(() => JSON.stringify((window as any).fixture.project.edits))
  const fit = await canvas.boundingBox(); const area = await viewport.boundingBox()
  expect(fit!.width).toBeLessThanOrEqual(area!.width + 1); expect(fit!.height).toBeLessThanOrEqual(area!.height + 1)
  await expect(page.getByRole('button', { name: 'Center', exact: true })).toBeDisabled()
  await scale.selectOption('100')
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBe(1920)
  await expect.poll(() => canvas.evaluate(el => (el as HTMLCanvasElement).width)).toBe(3840)
  await scale.selectOption('200')
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBe(3840)
  await expect.poll(() => canvas.evaluate(el => (el as HTMLCanvasElement).width)).toBe(4096)
  await viewport.focus()
  await viewport.press('Home')
  await expect.poll(() => viewport.evaluate(el => el.scrollLeft + el.scrollTop)).toBe(0)
  await viewport.press('ArrowRight'); await viewport.press('ArrowDown')
  await expect.poll(() => viewport.evaluate(el => [el.scrollLeft, el.scrollTop])).toEqual([40, 40])
  await viewport.press('Space')
  expect(await page.evaluate(() => (window as any).fixture.events.keys)).toEqual([' '])
  await viewport.press('End')
  expect(await viewport.evaluate(el => el.scrollLeft)).toBeGreaterThan(1000)
  await page.getByRole('button', { name: 'Center', exact: true }).click()
  await expect.poll(() => viewport.evaluate(el => Math.abs(el.scrollLeft - (el.scrollWidth - el.clientWidth) / 2))).toBeLessThanOrEqual(1)
  await scale.selectOption('fit')
  await expect.poll(() => viewport.evaluate(el => el.scrollLeft + el.scrollTop)).toBe(0)
  expect(await page.evaluate(() => JSON.stringify((window as any).fixture.project.edits))).toBe(edits)
  expect(await page.evaluate(() => (window as any).fixture.events.errors)).toEqual([])
  expect(errors).toEqual([])
  await page.screenshot({ path:'artifacts/ui/preview-scale-fit-dark.png' })
})

test('a cropped and zoomed source keeps mask coordinates after magnification and scrolling', async ({ page }) => {
  await harness(page)
  await page.evaluate(() => {
    const f = (window as any).fixture
    f.project.edits.crop = { x:.1,y:.1,width:.8,height:.8 }
    f.project.edits.zooms = [{ id:'zoom',start:0,end:10,scale:2,x:.5,y:.5,followCursor:false }]
    f.state.drawing = true
  })
  await page.getByLabel('Preview scale', { exact: true }).selectOption('200')
  const viewport = page.getByRole('region', { name: 'Video preview viewport' })
  await viewport.evaluate(el => { el.scrollLeft = 1450; el.scrollTop = 700 })
  const box = (await viewport.boundingBox())!
  const from = { x:box.x + 180,y:box.y + 150 }; const to = { x:from.x + 180,y:from.y + 100 }
  const expected = await page.evaluate(({ from, to }) => {
    const f = (window as any).fixture; const g = f.geometry(); const b = document.querySelector('canvas')!.getBoundingClientRect()
    const point = (p: {x:number;y:number}) => ({ x:(g.sx + (((p.x-b.left)*g.width/b.width-g.fx)/g.fw)*g.sw)/f.project.width,y:(g.sy + (((p.y-b.top)*g.height/b.height-g.fy)/g.fh)*g.sh)/f.project.height })
    const a = point(from); const z = point(to)
    return { x:a.x,y:a.y,width:z.x-a.x,height:z.y-a.y }
  }, { from, to })
  await page.mouse.move(from.x, from.y); await page.mouse.down(); await page.mouse.move(to.x, to.y, { steps:5 }); await page.mouse.up()
  await expect.poll(() => page.evaluate(() => (window as any).fixture.events.regions.length)).toBe(1)
  const actual = await page.evaluate(() => (window as any).fixture.events.regions[0])
  for (const key of ['x','y','width','height'] as const) expect(actual[key]).toBeCloseTo(expected[key], 6)
  expect(actual.x).toBeGreaterThan(.3); expect(actual.x + actual.width).toBeLessThan(.7)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.redactions = [{ ...f.events.regions[0],id:'mask',start:0,end:12,mode:'solid',strength:12 }] })
  await expect.poll(() => page.locator('canvas').evaluate((el, point) => {
    const canvas = el as HTMLCanvasElement; const box = canvas.getBoundingClientRect()
    return Array.from(canvas.getContext('2d')!.getImageData((point.x-box.left)*canvas.width/box.width,(point.y-box.top)*canvas.height/box.height,1,1).data)
  }, { x:(from.x+to.x)/2,y:(from.y+to.y)/2 })).toEqual([0,0,0,255])
  await page.screenshot({ path:'artifacts/ui/preview-scale-200-mask.png' })
})

test('controls remain usable in a narrow light-theme viewport and portrait Fit stays visible', async ({ page }) => {
  await harness(page)
  await page.setViewportSize({ width:480,height:800 })
  await page.evaluate(() => { document.documentElement.dataset.theme = 'light'; (window as any).fixture.project.edits.aspectRatio = '9:16' })
  const scale = page.getByLabel('Preview scale', { exact: true })
  await scale.focus(); await scale.selectOption('100'); await scale.selectOption('fit')
  const canvas = (await page.getByLabel('Edited video preview').boundingBox())!
  const viewport = (await page.getByRole('region', { name:'Video preview viewport' }).boundingBox())!
  expect(canvas.x).toBeGreaterThanOrEqual(viewport.x - 1)
  expect(canvas.y + canvas.height).toBeLessThanOrEqual(viewport.y + viewport.height + 1)
  expect(canvas.width / canvas.height).toBeCloseTo(9 / 16, 3)
  const controls = await page.locator('.preview-tools').boundingBox()
  expect(controls!.x + controls!.width).toBeLessThanOrEqual(480)
  await page.screenshot({ path:'artifacts/ui/preview-scale-fit-light.png' })
})

test('mask drag after crop, zoom, and pan makes one undo entry and Escape restores it', async ({ page }) => {
  await harness(page)
  await page.evaluate(() => {
    const f=(window as any).fixture
    f.project.edits.crop={x:.1,y:.1,width:.8,height:.8}
    f.project.edits.zooms=[{id:'zoom',start:0,end:10,scale:2,x:.5,y:.5,followCursor:false}]
    f.project.edits.redactions=[{id:'mask',start:0,end:12,x:.45,y:.45,width:.06,height:.08,mode:'solid',strength:12}]
    f.resetHistory()
  })
  await page.getByLabel('Preview scale',{exact:true}).selectOption('200')
  await page.getByRole('button',{name:'Center',exact:true}).click()
  const original=await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])
  const point=await page.evaluate(()=>{
    const f=(window as any).fixture;const g=f.geometry();const b=document.querySelector('canvas')!.getBoundingClientRect()
    return {x:b.left+(g.fx+(.48*f.project.width-g.sx)/g.sw*g.fw)*b.width/g.width,y:b.top+(g.fy+(.49*f.project.height-g.sy)/g.sh*g.fh)*b.height/g.height,dx:80/b.width*g.width/g.fw*g.sw/f.project.width,dy:50/b.height*g.height/g.fh*g.sh/f.project.height}
  })
  await page.mouse.move(point.x,point.y);await page.mouse.down();await page.mouse.move(point.x+80,point.y+50,{steps:8});await page.mouse.up()
  const moved=await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])
  expect(moved.x).toBeCloseTo(original.x+point.dx,5);expect(moved.y).toBeCloseTo(original.y+point.dy,5)
  expect(await page.evaluate(()=>(window as any).fixture.events.begins)).toBe(1)
  expect(await page.evaluate(()=>(window as any).fixture.events.ends)).toEqual([true])
  expect(await page.evaluate(()=>(window as any).fixture.events.selections)).toEqual(['mask'])
  expect(await page.evaluate(()=>(window as any).fixture.undo())).toBe(true)
  expect(await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])).toEqual(original)
  expect(await page.evaluate(()=>(window as any).fixture.undo())).toBe(false)
  const move=page.getByRole('button',{name:'Move mask',exact:true});const box=(await move.boundingBox())!
  await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();await page.mouse.move(box.x+box.width/2+70,box.y+box.height/2,{steps:4});await page.keyboard.press('Escape');await page.mouse.up()
  expect(await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])).toEqual(original)
  expect(await page.evaluate(()=>(window as any).fixture.events.ends)).toEqual([true,false])
  expect(await page.evaluate(()=>(window as any).fixture.undo())).toBe(false)
  const resize=page.getByRole('button',{name:'Resize mask bottom right',exact:true});await resize.focus();await resize.press('Shift+ArrowRight')
  const resized=await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])
  expect(resized.width).toBeCloseTo(original.width+10/960,6)
  expect(resized.x).toBe(original.x)
  await page.evaluate(()=>(window as any).fixture.state.disabled=true)
  await expect(move).toBeHidden()
  await page.mouse.move(point.x,point.y);await page.mouse.down();await page.mouse.move(point.x+50,point.y+30);await page.mouse.up()
  expect(await page.evaluate(()=>(window as any).fixture.project.edits.redactions[0])).toEqual(resized)
})

test('camera click, move, resize, keyboard movement, and pointer cancellation preserve output coordinates',async({page})=>{
  await harness(page)
  await page.evaluate(()=>{
    const f=(window as any).fixture;f.project.cameraPath='/viewport-source.mp4'
    f.project.edits.camera={enabled:true,x:.35,y:.3,width:.2,shape:'rectangle',mirror:false};f.resetHistory()
  })
  await expect.poll(()=>page.locator('video[data-track="webcam recording"]').evaluate(el=>(el as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(2)
  const canvas=page.getByLabel('Edited video preview');const c=(await canvas.boundingBox())!
  const start={x:c.x+c.width*.45,y:c.y+c.height*.4}
  await page.mouse.move(start.x,start.y);await page.mouse.down();await page.mouse.move(start.x+30,start.y+20,{steps:5});await page.mouse.up()
  const moved=await page.evaluate(()=>(window as any).fixture.project.edits.camera)
  expect(moved.x).toBeCloseTo(.35+30/c.width,3);expect(moved.y).toBeCloseTo(.3+20/c.height,3)
  expect(await page.evaluate(()=>(window as any).fixture.events.selections)).toEqual(['camera'])
  const resize=page.getByRole('button',{name:'Resize camera bottom right',exact:true});const r=(await resize.boundingBox())!
  await page.mouse.move(r.x+r.width/2,r.y+r.height/2);await page.mouse.down();await page.mouse.move(r.x+r.width/2+40,r.y+r.height/2+22.5,{steps:6});await page.mouse.up()
  const resized=await page.evaluate(()=>(window as any).fixture.project.edits.camera)
  expect(resized.width).toBeCloseTo(moved.width+40/c.width,2)
  expect(await page.evaluate(()=>(window as any).fixture.events.ends)).toEqual([true,true])
  const move=page.getByRole('button',{name:'Move camera',exact:true});await move.focus();await move.press('Shift+ArrowDown')
  const nudged=await page.evaluate(()=>(window as any).fixture.project.edits.camera)
  expect(nudged.y).toBeCloseTo((Math.round(resized.y*1080)+10)/1080,6)
  const box=(await move.boundingBox())!;await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();await page.mouse.move(box.x+box.width/2+25,box.y+box.height/2+25)
  await canvas.dispatchEvent('pointercancel',{pointerId:1});await page.mouse.up()
  expect(await page.evaluate(()=>(window as any).fixture.project.edits.camera)).toEqual(nudged)
  expect(await page.evaluate(()=>(window as any).fixture.events.ends)).toEqual([true,true,true,false])
  await page.screenshot({path:'artifacts/ui/preview-camera-handles.png'})
  await page.evaluate(()=>{
    const f=(window as any).fixture;const cam=f.project.edits.camera
    f.project.edits.redactions=[{id:'overlap',start:0,end:12,x:cam.x-.05,y:cam.y-.05,width:cam.width+.1,height:cam.width+.1,mode:'solid',strength:12}]
    f.state.selectedCamera=false;f.state.selectedRegion='overlap'
  })
  const position=await page.getByLabel('Edited video preview').boundingBox()
  const cam=await page.evaluate(()=>(window as any).fixture.project.edits.camera)
  await page.mouse.click(position!.x+(cam.x+cam.width/2)*position!.width,position!.y+(cam.y+cam.width/2)*position!.height)
  await expect(page.getByRole('button',{name:'Move camera',exact:true})).toBeVisible()
})

test('release stops loaded media and prevents playback or source resurrection before unmount',async({page})=>{
  await harness(page)
  await page.evaluate(()=>{const f=(window as any).fixture;f.project.cameraPath='/viewport-source.mp4';f.project.edits.camera.enabled=true;f.project.microphonePath='/viewport-audio.wav';f.project.systemAudioPath='/viewport-audio.wav';f.project.edits.music=[{id:'music',name:'Release test',path:'/viewport-audio.wav',sourceDuration:12,start:0,sourceStart:0,duration:12,volume:.5,fadeIn:0,fadeOut:0}]})
  await expect.poll(()=>page.locator('video[data-track="webcam recording"]').evaluate(el=>(el as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(2)
  await page.evaluate(async()=>{const f=(window as any).fixture;await f.preview().play();await f.preview().release();await f.preview().release();await f.preview().play();f.preview().seek(3);f.project.edits.background='#303030'})
  const state=await page.locator('video,audio').evaluateAll(elements=>elements.map(el=>{const media=el as HTMLMediaElement;return{src:media.getAttribute('src'),network:media.networkState,ready:media.readyState,paused:media.paused}}))
  expect(state).toHaveLength(5)
  for(const media of state){expect(media.src).toBeNull();expect(media.ready).toBe(0);expect(media.paused).toBe(true);expect([0,3]).toContain(media.network)}
  expect(await page.evaluate(()=>(window as any).fixture.events.errors)).toEqual([])
})


async function frameDriver(page: Page, fallback = false) {
  await page.addInitScript(({ fallback }) => {
    const w = window as any; let sequence = 0
    const nativeRequest = HTMLVideoElement.prototype.requestVideoFrameCallback; const nativeCancel = HTMLVideoElement.prototype.cancelVideoFrameCallback
    w.__compositions = 0; w.__videoCallbacks = new Map<number, { video: HTMLVideoElement; callback: VideoFrameRequestCallback }>()
    const transform = CanvasRenderingContext2D.prototype.setTransform
    CanvasRenderingContext2D.prototype.setTransform = function (...args: any[]) { if (this.canvas.classList.contains('preview-canvas')) w.__compositions++; return (transform as any).apply(this, args) } as typeof transform
    Object.defineProperty(HTMLVideoElement.prototype, 'requestVideoFrameCallback', { configurable: true, value: fallback ? undefined : function (this: HTMLVideoElement, callback: VideoFrameRequestCallback) { const id = ++sequence; const item = { video: this, callback, native: null as number | null }; w.__videoCallbacks.set(id, item); if (this.paused) item.native = nativeRequest.call(this, (now, metadata) => { if (!w.__videoCallbacks.has(id) || !this.paused || w.__acceptPaused === false) return; w.__videoCallbacks.delete(id); callback(now, metadata) }); return id } })
    Object.defineProperty(HTMLVideoElement.prototype, 'cancelVideoFrameCallback', { configurable: true, value: fallback ? undefined : function (id: number) { const item = w.__videoCallbacks.get(id); if (item?.native != null) nativeCancel.call(item.video, item.native); w.__videoCallbacks.delete(id) } })
    w.__present = (track: string, count: number, mediaTime?: number) => { for (const [id, item] of [...w.__videoCallbacks.entries()] as Array<[number, { video: HTMLVideoElement; callback: VideoFrameRequestCallback }]>) if (item.video.dataset.track === track) { w.__videoCallbacks.delete(id); item.callback(performance.now(), { presentedFrames: count, mediaTime: mediaTime ?? item.video.currentTime } as VideoFrameCallbackMetadata) } }
  }, { fallback })
}

test('presentation scheduler coalesces edits and camera frames without redrawing every clock tick', async ({ page }) => {
  await frameDriver(page); await harness(page)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.cameraPath = '/viewport-source.mp4'; f.project.edits.camera.enabled = true })
  await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => (element as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(2)
  await page.evaluate(async () => { await (window as any).fixture.preview().play() })
  await expect.poll(() => page.evaluate(() => (window as any).__videoCallbacks.size)).toBe(2)
  const video = page.locator('video[data-track="screen recording"]'); const start = await video.evaluate(element => (element as HTMLVideoElement).currentTime)
  const baseline = await page.evaluate(() => { (window as any).__acceptPaused = false; return (window as any).__compositions })
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(start + .2)
  expect(await page.evaluate(() => (window as any).__compositions - 0)).toBeLessThanOrEqual(baseline + 1)
  const beforeCamera = await page.evaluate(() => (window as any).__compositions)
  await page.evaluate(() => (window as any).__present('webcam recording', 1)); await expect.poll(() => page.evaluate(() => (window as any).__compositions)).toBe(beforeCamera + 1)
  await page.evaluate(() => { (window as any).fixture.preview().pause(); (window as any).__compositions = 0; for (let i = 0; i < 40; i++) (window as any).fixture.project.edits.padding = 40 + i })
  await expect.poll(() => page.evaluate(() => (window as any).__compositions)).toBe(1)
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  expect(await page.evaluate(() => (window as any).__compositions)).toBe(1)
})

test('presentation scheduler preserves timed privacy transitions even with no video callback', async ({ page }) => {
  await frameDriver(page); await harness(page)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.padding = 0; f.project.edits.cornerRadius = 0; f.project.edits.redactions = [{ id: 'timed', start: 1.3, end: 1.7, x: 0, y: 0, width: 1, height: 1, mode: 'solid', strength: 12 }] })
  const center = () => page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data].slice(0, 3))
  await page.evaluate(async () => { await (window as any).fixture.preview().play() })
  await expect.poll(() => center()).toEqual([0, 0, 0])
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(1.8)
  await expect.poll(() => center()).not.toEqual([0, 0, 0])
  await page.evaluate(() => (window as any).fixture.preview().pause())
})

async function clickPreview(page: Page) {
  await harness(page)
  await page.route('**/viewport-clicks.mp4', route => {
    const body = readFileSync(clicksFixture); const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/); const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}) }, body: body.subarray(start, end + 1) })
  })
  await page.evaluate(() => { const f = (window as any).fixture; f.project.screenPath = '/viewport-clicks.mp4'; Object.assign(f.project.edits, { trimStart: 0, trimEnd: 12, videoClips: [], padding: 96, cornerRadius: 40, showCursor: true, showClicks: true, cursorSize: 2, cursorSmoothing: 1, zooms: [] }); f.project.cursor = [{ time: 0, x: .25, y: .5, pressed: false, visible: true, buttons: 0 }, { time: 1, x: .25, y: .5, pressed: true, visible: true, buttons: 1 }, { time: 1.05, x: .25, y: .5, pressed: false, visible: true, buttons: 0 }, { time: 1.1, x: .5, y: .5, pressed: true, visible: true, buttons: 2 }, { time: 1.15, x: .5, y: .5, pressed: false, visible: true, buttons: 0 }, { time: 1.2, x: .75, y: .5, pressed: true, visible: true }, { time: 1.21, x: .9, y: .6, pressed: true, visible: true }] })
  await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
}
async function seekClickPreview(page: Page, time: number) {
  await page.evaluate(time => (window as any).fixture.preview().seek(time), time)
  await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
  await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(video => video.currentTime)).toBeCloseTo(time, 5)
}
async function ringPixel(page: Page, x: number, onset: number, radiusScale = 1) {
  return page.evaluate(({ x, onset, radiusScale }) => {
    const f = (window as any).fixture; const g = f.geometry(); const canvas = document.querySelector('canvas.preview-canvas') as HTMLCanvasElement; const time = (document.querySelector('video') as HTMLVideoElement).currentTime
    const radius = (12 + 16 * (time - onset) / .4) * f.project.edits.cursorSize * radiusScale
    const px = (g.fx + (x * f.project.width - g.sx) / g.sw * g.fw - radius) / g.width * canvas.width; const py = (g.fy + (.5 * f.project.height - g.sy) / g.sh * g.fh) / g.height * canvas.height
    return [...canvas.getContext('2d')!.getImageData(Math.round(px), Math.round(py), 1, 1).data].slice(0, 3)
  }, { x, onset, radiusScale })
}
test('recorded clicks render distinct rings at raw positions and fade without repeating held presses', async ({ page }) => {
  await clickPreview(page)
  await seekClickPreview(page, 1.02); const left = await ringPixel(page, .25, 1); expect(left[0]).toBeGreaterThan(160); expect(left[0]).toBeGreaterThan(left[2] * 2)
  await seekClickPreview(page, 1.3); const outer = await ringPixel(page, .5, 1.1); const inner = await ringPixel(page, .5, 1.1, .64)
  for (const pixel of [outer, inner]) { expect(pixel[2]).toBeGreaterThan(80); expect(pixel[2]).toBeGreaterThan(pixel[0] * 1.6) }
  const neutral = await ringPixel(page, .75, 1.2); expect(Math.max(...neutral) - Math.min(...neutral)).toBeLessThan(4); expect(neutral[0]).toBeGreaterThan(100)
  await seekClickPreview(page, 1.599); expect(Math.max(...await ringPixel(page, .75, 1.2))).toBeLessThan(5)
  await seekClickPreview(page, 1.7); expect(Math.max(...await ringPixel(page, .75, 1.3))).toBeLessThan(5)
  await seekClickPreview(page, 1.02); await page.evaluate(() => { (window as any).fixture.project.edits.showClicks = false }); await expect.poll(() => ringPixel(page, .25, 1)).toEqual([0, 0, 0])
  await page.evaluate(() => { (window as any).fixture.project.edits.showClicks = true; (window as any).fixture.project.edits.showCursor = false }); await expect.poll(() => ringPixel(page, .25, 1)).toEqual([0, 0, 0])
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.showCursor = true; f.project.cursor = [{ time: 0, x: .5, y: .5, visible: true, pressed: true }] }); await seekClickPreview(page, 0); expect((await ringPixel(page, .5, 0))[0]).toBeGreaterThan(190)
  await seekClickPreview(page, .4); await expect.poll(() => ringPixel(page, .5, 0)).toEqual([0, 0, 0])
})

test('click rings remain inside the final screen viewport and never tint background or cross cut gaps', async ({ page }) => {
  await clickPreview(page)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.crop = { x: .2, y: .2, width: .6, height: .6 }; f.project.edits.zooms = [{ id: 'zoom', start: 0, end: 4, scale: 1.5, x: .5, y: .5, followCursor: false }]; f.project.edits.backgroundMode = 'videoBlur' })
  await seekClickPreview(page, 1.3); const right = await ringPixel(page, .5, 1.1); expect(right[2]).toBeGreaterThan(right[0] * 1.6)
  expect(await page.locator('canvas.preview-canvas').evaluate(canvas => { const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data; let amber = 0; for (let i = 0; i < pixels.length; i += 4) if (pixels[i] > 20 && pixels[i] > pixels[i + 1] * 1.1 && pixels[i + 1] > pixels[i + 2] * 1.5) amber++; return amber })).toBe(0)
  const outside = await page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(2, 2, 1, 1).data]); expect(outside).toEqual([0, 0, 0, 255])
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.crop = null; f.project.edits.zooms = []; f.project.edits.videoClips = [{ id: 'before', start: 0, end: 1.09 }, { id: 'after', start: 1.11, end: 12 }] })
  await seekClickPreview(page, 1.13); await expect.poll(() => ringPixel(page, .5, 1.1)).toEqual([0, 0, 0]); await expect.poll(() => ringPixel(page, .25, 1)).toEqual([0, 0, 0])
  await page.evaluate(() => { (window as any).fixture.project.edits.videoClips = [] }); await seekClickPreview(page, 1.13); expect((await ringPixel(page, .5, 1.1))[2]).toBeGreaterThan(120)
  await page.evaluate(() => { (window as any).fixture.state.originalFrame = true }); await expect.poll(() => ringPixel(page, .5, 1.1)).toEqual([0, 0, 0]); await page.evaluate(() => { (window as any).fixture.state.originalFrame = false })
  await page.evaluate(() => { const f = (window as any).fixture; f.project.cursor = [{ time: 0, x: .5, y: .5, pressed: false, visible: true, buttons: 0 }, { time: 1.1, x: .5, y: .5, pressed: true, visible: true, buttons: 2 }, { time: 1.14, x: .5, y: .5, pressed: true, visible: false, buttons: 2 }, { time: 1.15, x: .5, y: .5, pressed: true, visible: true, buttons: 2 }] }); await seekClickPreview(page, 1.16); await expect.poll(() => ringPixel(page, .5, 1.1)).toEqual([0, 0, 0])
  await page.evaluate(() => { (window as any).fixture.project.edits.camera.layout = 'fullscreen' }); await expect.poll(() => page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data])).toEqual([32, 32, 32, 255])
})

test('stationary click animation invalidates static frames only during its active duration', async ({ page }) => {
  await frameDriver(page); await harness(page)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.edits.showCursor = true; f.project.edits.showClicks = true; f.project.cursor = [{ time: 0, x: .5, y: .5, pressed: false, visible: true }, { time: 1.3, x: .5, y: .5, pressed: true, visible: true, buttons: 1 }, { time: 1.31, x: .5, y: .5, pressed: false, visible: true }] })
  await page.evaluate(async () => { await (window as any).fixture.preview().play(); (window as any).__acceptPaused = false; (window as any).__compositions = 0 })
  await expect.poll(() => page.locator('video').evaluate(video => video.currentTime)).toBeGreaterThan(1.6)
  expect(await page.evaluate(() => (window as any).__compositions)).toBeGreaterThan(4)
  await expect.poll(() => page.locator('video').evaluate(video => video.currentTime)).toBeGreaterThan(1.9)
  const count = await page.evaluate(() => (window as any).__compositions)
  await expect.poll(() => page.locator('video').evaluate(video => video.currentTime)).toBeGreaterThan(2.2)
  expect(await page.evaluate(() => (window as any).__compositions)).toBe(count)
  await page.evaluate(() => (window as any).fixture.preview().pause())
})

test('presentation scheduler uses runtime RAF fallback and releases all callbacks with media', async ({ page }) => {
  await frameDriver(page, true); await harness(page)
  await page.evaluate(async () => { await (window as any).fixture.preview().play(); (window as any).__compositions = 0 })
  await expect.poll(() => page.evaluate(() => (window as any).__compositions)).toBeGreaterThan(3)
  await page.evaluate(async () => { await (window as any).fixture.preview().release(); (window as any).__compositions = 0 })
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  expect(await page.evaluate(() => (window as any).__compositions)).toBe(0)
})

test('camera visibility follows its own clock and accepts the frame active before an interval starts', async ({ page }) => {
  await frameDriver(page); await harness(page)
  await page.evaluate(() => { const f = (window as any).fixture; f.project.cameraPath = '/viewport-source.mp4'; f.project.cameraIntervals = [{ start: 1, end: 1.15 }]; f.project.edits.camera.enabled = true; f.project.edits.camera.layout = 'split' })
  await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => (element as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(2)
  await page.evaluate(async () => { await (window as any).fixture.preview().play() })
  await expect.poll(() => page.evaluate(() => (window as any).__videoCallbacks.size)).toBe(2)
  await page.evaluate(() => {
    const w = window as any; w.__cameraClock = 1.1
    Object.defineProperty(document.querySelector('video[data-track="screen recording"]'), 'currentTime', { configurable: true, get: () => 1.1, set: () => {} })
    Object.defineProperty(document.querySelector('video[data-track="webcam recording"]'), 'currentTime', { configurable: true, get: () => w.__cameraClock, set: () => {} })
    w.__present('webcam recording', 1, .98)
  })
  await expect(page.getByText('Camera unavailable', { exact: true })).toBeHidden()
  await page.evaluate(() => { (window as any).__cameraClock = 1.16 })
  await expect(page.getByText('Camera unavailable', { exact: true })).toBeVisible()
  const background = await page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width * .9, canvas.height * .5, 1, 1).data])
  expect(background).toEqual([32, 32, 32, 255])
  await page.evaluate(() => { const w = window as any; w.__cameraClock = 1.1; w.__present('webcam recording', 2, 1.15) })
  await expect(page.getByText('Camera unavailable', { exact: true })).toBeVisible()
  await page.evaluate(() => (window as any).fixture.preview().pause())
})

test('paused fractional seeks present the same CFR and sparse VFR frame without waiting for another frame', async ({ page }) => {
  await harness(page)
  await page.route('**/viewport-sparse.mp4', route => {
    const body = readFileSync(sparseFixture); const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/)
    const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}) }, body: body.subarray(start, end + 1) })
  })
  const timings: number[] = []
  for (const sparse of [false, true]) {
    await page.evaluate(sparse => { const f = (window as any).fixture; f.project.fps = 60; f.project.width = sparse ? 320 : 960; f.project.height = sparse ? 180 : 540; f.project.screenPath = sparse ? '/viewport-sparse.mp4' : '/viewport-source.mp4'; f.project.cameraPath = f.project.screenPath; f.project.cameraOffsetSeconds = .017; f.project.edits.camera.enabled = true; f.project.edits.camera.layout = 'split' }, sparse)
    await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
    await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => element.readyState)).toBeGreaterThanOrEqual(2)
    for (const time of [1.201, 1.205, 1.21, 1.215, 1.201]) {
      const start = Date.now(); await page.evaluate(time => (window as any).fixture.preview().seek(time), time)
      await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden({ timeout: 1500 }); timings.push(Date.now() - start)
      await expect.poll(() => page.locator('video[data-track="screen recording"]').evaluate(element => element.currentTime)).toBeCloseTo(time, 5)
      await expect.poll(() => page.locator('video[data-track="webcam recording"]').evaluate(element => element.currentTime)).toBeCloseTo(time + .017, 5)
      await expect(page.getByText('Camera unavailable', { exact: true })).toBeHidden()
      if (sparse) {
        const pixels = await page.locator('canvas.preview-canvas').evaluate(canvas => { const ctx = canvas.getContext('2d')!; return [.25, .85].map(x => [...ctx.getImageData(canvas.width * x, canvas.height * .5, 1, 1).data]) })
        for (const pixel of pixels) expect(pixel[2]).toBeGreaterThan(220)
      }
    }
  }
  expect(await page.evaluate(() => (window as any).fixture.events.errors)).toEqual([])
  console.log('Paused CFR/VFR seek milliseconds', JSON.stringify(timings))
})

for (const runtime of ['unavailable', 'security-error'] as const) test(`pausing at the uncertain cut edge cannot reopen a frame before a verified seek (${runtime})`, async ({ page }) => {
  await page.addInitScript(runtime => { Object.defineProperty(window, 'VideoFrame', { configurable: true, value: runtime === 'unavailable' ? undefined : function () { throw new DOMException('Origin is not clean', 'SecurityError') } }) }, runtime)
  await frameDriver(page); await harness(page)
  await page.evaluate(async () => { await (window as any).fixture.preview().play() })
  await page.evaluate(() => {
    const source = document.querySelector('video') as HTMLVideoElement
    Object.defineProperty(source, 'currentTime', { configurable: true, get: () => 11.99, set: () => {} })
    ;(window as any).fixture.preview().pause()
    ;(window as any).fixture.project.edits.padding = 60
  })
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const background = await page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width * .5, canvas.height * .5, 1, 1).data])
  expect(background).toEqual([32, 32, 32, 255])
})

for (const allowed of [true, false]) test(`cross-origin video ${allowed ? 'renders with anonymous CORS' : 'rejects a denied origin without retaining old pixels'}`, async ({ page }) => {
  await harness(page)
  let allowRequests = allowed
  const pageOrigin = new URL(page.url()).origin
  const foreignSource = new URL('/foreign-source.mp4', pageOrigin)
  foreignSource.hostname = foreignSource.hostname === 'localhost' ? '127.0.0.1' : 'localhost'
  await page.route(foreignSource.href, route => {
    const body = readFileSync(fixture); const range = route.request().headers().range?.match(/bytes=(\d+)-(\d*)/)
    const start = range ? Number(range[1]) : 0; const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1
    return route.fulfill({ status: range ? 206 : 200, contentType: 'video/mp4', headers: { 'Accept-Ranges': 'bytes', 'Content-Length': String(end - start + 1), ...(range ? { 'Content-Range': `bytes ${start}-${end}/${body.length}` } : {}), 'Access-Control-Allow-Origin': allowRequests ? pageOrigin : 'https://denied.invalid' }, body: body.subarray(start, end + 1) })
  })
  await page.evaluate(source => { (window as any).fixture.project.screenPath = source }, foreignSource.href)
  expect(await page.locator('video').getAttribute('crossorigin')).toBe('anonymous')
  if (allowed) {
    await expect.poll(() => page.locator('video').evaluate(element => element.readyState)).toBeGreaterThanOrEqual(2)
    await page.evaluate(() => (window as any).fixture.preview().seek(11.98))
    await expect.poll(() => page.locator('video').evaluate(element => element.currentTime)).toBeCloseTo(11.98, 3)
    await expect(page.getByText('Seeking edited video...', { exact: true })).toBeHidden()
    expect(await page.evaluate(() => (window as any).fixture.events.errors)).toEqual([])
    const pixel = await page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width * .5, canvas.height * .5, 1, 1).data])
    expect(pixel).not.toEqual([32, 32, 32, 255])
  } else {
    await expect.poll(() => page.evaluate(() => (window as any).fixture.events.errors.join(' '))).toContain('Cannot preview screen recording')
    const pixel = await page.locator('canvas.preview-canvas').evaluate(canvas => [...canvas.getContext('2d')!.getImageData(canvas.width * .5, canvas.height * .5, 1, 1).data])
    expect(pixel).toEqual([32, 32, 32, 255])
    await expect(page.getByText('Loading video...', { exact: true })).toBeHidden()
    await expect(page.getByText('Video unavailable. Press Play to retry.')).toBeVisible()
    allowRequests = true
    await page.evaluate(() => (window as any).fixture.preview().play())
    await expect(page.locator('.preview-loading')).toBeHidden()
    await expect.poll(() => page.locator('video').evaluate(el => el.currentTime)).toBeGreaterThan(1.3)
    await page.evaluate(() => { (window as any).fixture.project.screenPath = '/viewport-source.mp4' })
    await expect.poll(() => page.locator('video').evaluate(el => el.readyState)).toBeGreaterThanOrEqual(2)
    await expect(page.locator('.preview-loading')).toBeHidden()
    await page.evaluate(() => (window as any).fixture.preview().play())
    await expect.poll(() => page.locator('video').evaluate(el => el.currentTime)).toBeGreaterThan(1.3)
  }
})
