<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { command, errorText } from '../api'
import { appearanceSettings, builtInPresets } from '../appearance'
import type { AppearancePreset, AppearanceSettings, Project } from '../types'

const props = defineProps<{ project: Project }>()
const emit = defineEmits<{ apply: [AppearanceSettings]; newCameraRecording: [] }>()
const presets = ref<AppearancePreset[]>([])
const selectedId = ref(''); const name = ref(''); const error = ref(''); const notice = ref('')
const pending = ref(''); const loadFailed = ref(false); const saving = ref(false)
const nameInput = ref<HTMLInputElement>(); const saveButton = ref<HTMLButtonElement>(); const deleteButton = ref<HTMLButtonElement>(); const confirmation = ref<HTMLDialogElement>()
const selected = computed(() => [...builtInPresets, ...presets.value].find(preset => preset.id === selectedId.value))
const custom = computed(() => presets.value.find(preset => preset.id === selectedId.value))
const cameraRequired = computed(() => !props.project.cameraPath && selected.value?.settings.camera.layout === 'fullscreen')
let generation = 0; let disposed = false
async function load() {
  const request = ++generation; pending.value = 'Loading presets'; error.value = ''
  try { const value = await command<AppearancePreset[]>('list_appearance_presets'); if (disposed || request !== generation) return; presets.value = value; loadFailed.value = false }
  catch (failure) { if (!disposed && request === generation) { error.value = errorText(failure); loadFailed.value = true } }
  finally { if (!disposed && request === generation) pending.value = '' }
}
function apply() {
  if (!selected.value || pending.value || cameraRequired.value) return
  if (!loadFailed.value) error.value = ''
  notice.value = ''; emit('apply', appearanceSettings(selected.value.settings))
}
async function beginSave() { saving.value = true; name.value = ''; notice.value = ''; error.value = ''; await nextTick(); nameInput.value?.focus() }
function cancelSave() { if (pending.value) return; saving.value = false; void nextTick(() => saveButton.value?.focus()) }
async function save() {
  if (!name.value.trim() || pending.value || loadFailed.value) return
  const request = ++generation; pending.value = 'Saving preset'; error.value = ''
  const settings = appearanceSettings(props.project.edits)
  try {
    const preset = await command<AppearancePreset>('save_appearance_preset', { name: name.value.trim(), settings })
    if (disposed || request !== generation) return
    presets.value = [...presets.value, preset]; selectedId.value = preset.id; saving.value = false; notice.value = 'Preset saved.'
    void nextTick(() => saveButton.value?.focus())
  } catch (failure) { if (!disposed && request === generation) error.value = errorText(failure) }
  finally { if (!disposed && request === generation) pending.value = '' }
}
function askDelete() { if (custom.value && !pending.value) { error.value = ''; confirmation.value?.showModal() } }
function closeDelete() { if (pending.value) return; confirmation.value?.close(); void nextTick(() => deleteButton.value?.focus()) }
async function remove() {
  const target = custom.value; if (!target || pending.value) return
  const request = ++generation; pending.value = 'Deleting preset'; error.value = ''
  try {
    await command('delete_appearance_preset', { id: target.id })
    if (disposed || request !== generation) return
    presets.value = presets.value.filter(preset => preset.id !== target.id); selectedId.value = ''; confirmation.value?.close(); notice.value = 'Preset deleted.'
    void nextTick(() => saveButton.value?.focus())
  } catch (failure) { if (!disposed && request === generation) error.value = errorText(failure) }
  finally { if (!disposed && request === generation) pending.value = '' }
}
watch(() => props.project.id, () => { saving.value = false; confirmation.value?.close(); error.value = ''; notice.value = '' })
onMounted(() => void load())
onBeforeUnmount(() => { disposed = true; generation++; confirmation.value?.close() })
</script>

<template>
  <section class="appearance-presets" aria-label="Appearance presets">
    <label for="appearance-preset">Appearance preset</label>
    <div class="preset-choice"><select id="appearance-preset" v-model="selectedId" :disabled="!!pending"><option value="">Choose preset</option><optgroup label="Built-in"><option v-for="preset in builtInPresets" :key="preset.id" :value="preset.id">{{ preset.name }}</option></optgroup><optgroup v-if="presets.length" label="Custom"><option v-for="preset in presets" :key="preset.id" :value="preset.id">{{ preset.name }}</option></optgroup></select><button type="button" :disabled="!selected || !!pending || cameraRequired" @click="apply">Apply</button></div>
    <p v-if="cameraRequired" class="field-help">This preset requires recorded camera video. <button type="button" class="text-button" @click="emit('newCameraRecording')">New recording with camera</button>, or choose another preset.</p>
    <div class="tool-actions"><button ref="saveButton" type="button" class="small-button" :disabled="!!pending || loadFailed || presets.length >= 50" @click="beginSave">Save current...</button><button v-if="custom" ref="deleteButton" type="button" class="small-button danger-text" :disabled="!!pending" @click="askDelete">Delete preset...</button></div>
    <p v-if="presets.length >= 50" class="field-help">50 custom presets saved. Delete a preset before saving another.</p>
    <form v-if="saving" class="preset-save" @submit.prevent="save" @keydown.esc.stop.prevent="cancelSave"><label for="preset-name">Preset name</label><input id="preset-name" ref="nameInput" v-model="name" maxlength="120" :disabled="!!pending" autocomplete="off" /><div class="tool-actions"><button type="submit" :disabled="!name.trim() || !!pending">Save preset</button><button type="button" :disabled="!!pending" @click="cancelSave">Cancel</button></div></form>
    <p v-if="error && !confirmation?.open" role="alert" class="preset-error">{{ error }}</p><button v-if="loadFailed" type="button" class="small-button" :disabled="!!pending" @click="load">Retry presets</button>
    <p v-if="pending || notice" class="field-help" role="status">{{ pending || notice }}</p>
    <dialog ref="confirmation" class="preset-delete-dialog" aria-labelledby="delete-preset-title" @cancel.prevent="closeDelete"><h2 id="delete-preset-title">Delete appearance preset?</h2><p>{{ custom?.name }}</p><p>Existing projects keep their appearance.</p><p v-if="error" class="preset-error" role="alert">{{ error }}</p><div class="dialog-actions"><button type="button" autofocus :disabled="!!pending" @click="closeDelete">Cancel</button><button type="button" class="danger-text" :disabled="!!pending" @click="remove">{{ pending ? 'Deleting...' : 'Delete preset' }}</button></div></dialog>
  </section>
</template>

<style scoped>
.appearance-presets { padding-bottom: 16px; margin-bottom: 16px; border-bottom: 1px solid var(--line); }
.preset-choice { display: flex; align-items: center; gap: 6px; }.preset-choice select { min-width: 0; flex: 1; }.preset-choice button { padding-inline: 10px; }
.preset-save { padding-top: 12px; }.preset-error { color: var(--danger); line-height: 1.5; overflow-wrap: anywhere; }
.preset-delete-dialog { max-width: min(440px, calc(100vw - 32px)); padding: 24px; }.preset-delete-dialog>p { margin-block: 14px; overflow-wrap: anywhere; }
</style>
