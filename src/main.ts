import { createApp } from 'vue'
import './style.css'

const overlay = new URLSearchParams(location.search).get('overlay') === 'recording'
const entry = overlay ? import('./components/RecordingControls.vue') : import('./App.vue')
entry.then(({ default: component }) => createApp(component).mount('#app'))
