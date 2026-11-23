export class MusicDucker {
  constructor(rate = 48000) {
    this.attack = Math.exp(-1 / (rate * .010));
    this.envelopeRelease = Math.exp(-1 / (rate * .150));
    this.gainRelease = Math.exp(-1 / (rate * .250));
    this.configure(false, 12);
    this.reset();
  }

  configure(enabled, amountDb) {
    this.enabled = enabled;
    this.floor = 10 ** (-Math.max(0, Math.min(24, amountDb)) / 20);
    if (!enabled) this.reset();
  }

  reset() { this.envelope = 0; this.gain = 1; }

  process(micLeft, micRight) {
    if (!this.enabled) return 1;
    const power = (micLeft * micLeft + micRight * micRight) / 2;
    this.envelope = power + (this.envelope - power) * (power > this.envelope ? this.attack : this.envelopeRelease);
    const activity = Math.max(0, Math.min(1, (Math.sqrt(this.envelope) - .01) / (10 ** (-30 / 20) - .01)));
    const target = 1 - activity * (1 - this.floor);
    this.gain = target + (this.gain - target) * (target < this.gain ? this.attack : this.gainRelease);
    return this.gain;
  }
}

if (typeof registerProcessor === 'function') {
  class MusicMixer extends AudioWorkletProcessor {
    static get parameterDescriptors() {
      return [
        { name: 'active', defaultValue: 0, minValue: 0, maxValue: 1, automationRate: 'a-rate' },
        ...['microphoneVolume', 'systemVolume', 'musicVolume'].map(name => ({ name, defaultValue: 1, minValue: 0, maxValue: 1, automationRate: 'a-rate' })),
      ];
    }
    constructor() {
      super();
      this.ducker = new MusicDucker(sampleRate);
      this.musicEnabled = false;
      this.systemEnabled = false;
      this.monitor = 'mix';
      this.port.onmessage = ({ data }) => {
        if (data === 'reset') this.ducker.reset();
        else if ('monitor' in data) this.monitor = data.monitor === 'microphone' || data.monitor === 'system' ? data.monitor : 'mix';
        else {
          this.musicEnabled = !!data.enabled;
          this.systemEnabled = !!data.systemEnabled;
          this.ducker.configure(this.musicEnabled || this.systemEnabled, data.amountDb);
        }
      };
    }

    process(inputs, outputs, parameters) {
      const out = outputs[0];
      const mic = inputs[0] ?? []; const system = inputs[1] ?? []; const music = inputs[2] ?? [];
      const micVolume = parameters.microphoneVolume; const systemVolume = parameters.systemVolume; const musicVolume = parameters.musicVolume;
      for (let i = 0; i < out[0].length; i++) {
        if (!(parameters.active[parameters.active.length === 1 ? 0 : i] > 0)) { out[0][i] = 0; out[1][i] = 0; continue; }
        const micLevel = micVolume ? micVolume[micVolume.length === 1 ? 0 : i] : 1;
        const systemLevel = systemVolume ? systemVolume[systemVolume.length === 1 ? 0 : i] : 1;
        const musicLevel = musicVolume ? musicVolume[musicVolume.length === 1 ? 0 : i] : 1;
        const micInput = mic[0]?.[i] ?? 0;
        const left = Math.fround(micInput * micLevel); const right = Math.fround((mic[1]?.[i] ?? micInput) * micLevel);
        const gain = this.ducker.process(left, right);
        const systemGain = this.systemEnabled ? gain : 1;
        const musicGain = this.musicEnabled ? gain : 1;
        const micGain = this.monitor === 'system' ? 0 : 1;
        const monitoredSystemGain = this.monitor === 'microphone' ? 0 : systemGain;
        const monitoredMusicGain = this.monitor === 'mix' ? musicGain : 0;
        out[0][i] = left * micGain + Math.fround((system[0]?.[i] ?? 0) * systemLevel) * monitoredSystemGain + Math.fround((music[0]?.[i] ?? 0) * musicLevel) * monitoredMusicGain;
        out[1][i] = right * micGain + Math.fround((system[1]?.[i] ?? system[0]?.[i] ?? 0) * systemLevel) * monitoredSystemGain + Math.fround((music[1]?.[i] ?? music[0]?.[i] ?? 0) * musicLevel) * monitoredMusicGain;
      }
      return true;
    }
  }
  registerProcessor('screenrecord-music-mixer', MusicMixer);
}
