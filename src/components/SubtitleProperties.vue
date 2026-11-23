<script setup lang="ts">
import { computed, ref } from 'vue'
import { timeLabel } from '../editing'
import { nextCue, updateInterval } from '../tracks'
import { mergeCues, retimeCue, splitCue } from '../subtitle-editing'
import { projectedSubtitle } from '../subtitle-projection'
import type { Project, TrackSelection } from '../types'

const props = defineProps<{ project: Project; selection: TrackSelection | null; time: number }>()
const emit = defineEmits<{ select: [TrackSelection | null]; transcribe: []; exportSrt: []; importSrt: []; error: [string] }>()
const search = ref(''); const notice = ref(''); const textInput = ref<HTMLTextAreaElement>(); const caret = ref<{ id: string; offset: number }>()
const cue = computed(() => props.project.edits.subtitles.find(item => item.id === props.selection?.id))
const filtered = computed(() => props.project.edits.subtitles.filter(item => item.text.toLocaleLowerCase().includes(search.value.toLocaleLowerCase())))
const cueIndex = computed(() => filtered.value.findIndex(item => item.id === cue.value?.id))
const rawIndex = computed(() => props.project.edits.subtitles.findIndex(item => item.id === cue.value?.id))
const following = computed(() => props.project.edits.subtitles[rawIndex.value + 1])
const canSplit = computed(() => !!cue.value && props.time > cue.value.start && props.time < cue.value.end)
const canMerge = computed(() => !!cue.value && !!following.value && !!projectedSubtitle(props.project.edits, cue.value) && !!projectedSubtitle(props.project.edits, following.value))
const untimed = computed(() => props.project.edits.subtitles.filter(item => !item.words?.length).length)
function select(id: string) { caret.value = undefined; notice.value = ''; emit('select', { kind: 'subtitles', id }) }
function navigate(delta: number) { const item = filtered.value[cueIndex.value + delta]; if (item) select(item.id) }
function add() {
  if (props.project.edits.subtitles.length >= 10_000) { emit('error', 'The project already has 10,000 subtitle cues.'); return }
  const added = nextCue(props.project, props.time)
  if (!added) { emit('error', 'There is no free subtitle interval here. Move the playhead or adjust an existing cue.'); return }
  props.project.edits.subtitles = [...props.project.edits.subtitles, added].sort((a, b) => a.start - b.start); select(added.id)
}
function remove() { props.project.edits.subtitles = props.project.edits.subtitles.filter(item => item.id !== cue.value?.id); emit('select', null) }
function editText(event: Event) {
  if (!cue.value) return
  const input = event.target as HTMLTextAreaElement
  const text = input.value.split('\n').slice(0, 3).join('\n').slice(0, 500)
  if (cue.value.words?.length && text !== cue.value.text) notice.value = 'Text changed. This cue now uses plain timing; generate again to restore word timing.'
  cue.value.text = text; cue.value.words = []; input.value = text; caret.value = { id: cue.value.id, offset: input.selectionStart }
}
function timing(field: 'start' | 'end', event: Event) {
  if (!cue.value) return
  const proposed = { ...cue.value }; updateInterval(proposed, field, Number((event.target as HTMLInputElement).value), props.project.duration, props.project.edits.subtitles, .01)
  const result = retimeCue(cue.value, proposed.start, proposed.end)
  if (typeof result === 'string') emit('error', result); else Object.assign(cue.value, result)
  ;(event.target as HTMLInputElement).value = cue.value[field].toFixed(3)
}
function split() {
  if (!cue.value || props.project.edits.subtitles.length >= 10_000) return
  const result = splitCue(cue.value, props.time, caret.value?.id === cue.value.id ? caret.value.offset : undefined)
  if (typeof result === 'string') { emit('error', result); return }
  const items = [...props.project.edits.subtitles]; items.splice(rawIndex.value, 1, result.left, result.right); props.project.edits.subtitles = items; select(result.right.id)
  notice.value = result.estimated ? 'Text split was estimated from the playhead. Review both cues; word timing was not invented.' : result.left.words?.length ? 'Split at a word boundary.' : 'Split at the text caret. Both cues retain plain timing.'
}
function merge() {
  if (!cue.value || !following.value) return
  const left = projectedSubtitle(props.project.edits, cue.value); const right = projectedSubtitle(props.project.edits, following.value)
  if (!left || !right) { emit('error', 'Only cues with retained text can be merged. Restore footage or edit the cue first.'); return }
  const result = mergeCues(left, right)
  if (typeof result === 'string') { emit('error', result); return }
  const items = [...props.project.edits.subtitles]; items.splice(rawIndex.value, 2, result); props.project.edits.subtitles = items; select(result.id)
  notice.value = result.words?.length ? 'Merged retained words and their timings.' : 'Merged retained text. The combined cue uses plain timing because one or both cues had no word timing.'
}
</script>

<template>
  <section>
    <h3>Subtitles</h3><div class="tool-actions"><button type="button" class="small-button" :disabled="!project.microphonePath && !project.systemAudioPath" @click="emit('transcribe')">Generate subtitles</button><button type="button" class="small-button" @click="emit('importSrt')">Import SRT</button><button type="button" class="small-button" @click="add">Add subtitle</button><button type="button" class="small-button" :disabled="!project.edits.subtitles.length" @click="emit('exportSrt')">Export SRT</button></div>
    <p class="field-help">Imported SRT uses edited output time and replaces current cues. Undo restores them.</p>
    <p v-if="!project.microphonePath && !project.systemAudioPath" class="field-help">No recorded audio is available for generation. Import SRT or add captions manually.</p>
    <template v-if="project.edits.subtitles.length">
      <label for="subtitle-search">Search subtitle text</label><input id="subtitle-search" v-model="search" type="search" />
      <label for="subtitle-selection">Subtitle cue <output>{{ filtered.length }} cues</output></label><select id="subtitle-selection" :value="cue?.id ?? ''" :size="Math.min(6, Math.max(2, filtered.length + 1))" @change="select(($event.target as HTMLSelectElement).value)"><option disabled value="">Choose a cue</option><option v-for="item in filtered" :key="item.id" :value="item.id">{{ timeLabel(item.start, true) }} {{ item.text.slice(0, 50) }}</option></select>
      <div class="tool-actions"><button type="button" class="small-button" :disabled="cueIndex <= 0" @click="navigate(-1)">Previous cue</button><button type="button" class="small-button" :disabled="cueIndex >= filtered.length - 1" @click="navigate(1)">Next cue</button></div>
      <template v-if="cue">
        <label for="subtitle-text">Subtitle text</label><textarea id="subtitle-text" ref="textInput" :value="cue.text" rows="3" maxlength="500" @input="editText" @select="caret = { id: cue.id, offset: textInput?.selectionStart ?? 0 }" /><p class="field-help">Up to 3 lines and 500 characters. Text edits remove word timing.</p>
        <div class="tool-actions"><button type="button" class="small-button" :disabled="!canSplit || project.edits.subtitles.length >= 10_000" @click="split">Split at playhead</button><button type="button" class="small-button" :disabled="!canMerge" @click="merge">Merge next cue</button></div>
        <p class="field-help">{{ !canSplit ? 'Move the playhead inside this cue to split it.' : cue.words?.length ? 'Split follows the nearest word boundary.' : 'Split uses the text caret, or estimates the text boundary from the playhead.' }}</p>
        <div class="field-grid"><div><label for="subtitle-start">Subtitle start (s)</label><input id="subtitle-start" type="number" min="0" step=".01" :value="cue.start.toFixed(3)" @change="timing('start', $event)" /></div><div><label for="subtitle-end">Subtitle end (s)</label><input id="subtitle-end" type="number" min=".01" step=".01" :value="cue.end.toFixed(3)" @change="timing('end', $event)" /></div></div>
        <p v-if="(cue.sourceRanges?.length ?? 0) > 1" class="field-help">This cue spans separate source ranges. It stays hidden between those ranges.</p>
        <button type="button" class="danger-text full-width" @click="remove">Delete subtitle</button>
      </template>
    </template>
    <p v-if="notice" role="status" class="field-help">{{ notice }}</p>
    <div class="section-divider"></div><label class="check-label"><input v-model="project.edits.subtitleStyle.enabled" type="checkbox" />Show subtitles in video</label>
    <label class="check-label"><input v-model="project.edits.subtitleStyle.highlightWords" type="checkbox" />Highlight spoken words</label><p v-if="project.edits.subtitleStyle.highlightWords && untimed" class="field-help">{{ untimed }} {{ untimed === 1 ? 'cue has' : 'cues have' }} no word timing and {{ untimed === 1 ? 'displays' : 'display' }} without highlighting. Generate subtitles to obtain word timing.</p><template v-if="project.edits.subtitleStyle.highlightWords"><label for="subtitle-highlight">Word highlight color</label><input id="subtitle-highlight" v-model="project.edits.subtitleStyle.highlightColor" type="color" /></template>
    <label for="subtitle-size">Font size</label><input id="subtitle-size" v-model.number="project.edits.subtitleStyle.fontSize" type="range" min="12" max="96" step="1" /><div class="field-grid"><div><label for="subtitle-color">Text color</label><input id="subtitle-color" v-model="project.edits.subtitleStyle.color" type="color" /></div><div><label for="subtitle-background">Text background</label><input id="subtitle-background" v-model="project.edits.subtitleStyle.background" type="color" /></div></div><label for="subtitle-position">Position</label><select id="subtitle-position" v-model="project.edits.subtitleStyle.position"><option value="bottom">Bottom</option><option value="top">Top</option></select>
  </section>
</template>
