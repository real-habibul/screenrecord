/*
 * Screenrecord AudioWorklet adaptation of FFmpeg's af_alimiter.c (asc=0, level=0).
 * Original: https://github.com/FFmpeg/FFmpeg/blob/a35157abb9/libavfilter/af_alimiter.c
 * Copyright (C) 2001-2010 Krzysztof Foltman, Markus Schmidt,
 * Thor Harald Johansen and others; Copyright (c) 2015 Paul B Mahol.
 *
 * This module is free software: you can redistribute it and/or modify it under
 * the GNU Lesser General Public License, version 2.1 or (at your option) later.
 * It is distributed WITHOUT ANY WARRANTY, including MERCHANTABILITY or FITNESS
 * FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for details.
 * License: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
 * This readable module is distributed separately from the application bundle.
 */

export class LookaheadLimiter {
  constructor(rate = 48000) {
    this.rate = rate;
    this.limit = 0.95;
    this.size = Math.max(2, Math.floor(rate * 0.005) * 2);
    this.latencyFrames = this.size / 2 - 1;
    this.buffer = new Float64Array(this.size);
    this.nextPosition = new Int32Array(this.size);
    this.nextSlope = new Float64Array(this.size);
    this.reset();
  }

  reset() {
    this.buffer.fill(0);
    this.nextPosition.fill(-1);
    this.nextSlope.fill(0);
    this.position = 0;
    this.gain = 1;
    this.slope = 0;
    this.queueStart = 0;
    this.queueLength = 0;
    this.left = 0;
    this.right = 0;
  }

  process(left, right) {
    const size = this.size;
    const position = this.position;
    this.buffer[position] = left;
    this.buffer[position + 1] = right;
    const peak = Math.max(Math.abs(left), Math.abs(right));
    if (peak > this.limit) {
      const target = this.limit / peak;
      const releaseSlope = (1 - target) / (this.rate * 0.05);
      const attackSlope = (target - this.gain) / size * 2;
      if (attackSlope < this.slope) {
        this.slope = attackSlope;
        this.nextPosition[0] = position;
        this.nextPosition[1] = -1;
        this.nextSlope[0] = releaseSlope;
        this.queueLength = 1;
        this.queueStart = 0;
      } else {
        for (let item = this.queueStart; item < this.queueStart + this.queueLength; item++) {
          const index = item % size;
          const queuedPosition = this.nextPosition[index];
          const queuedPeak = queuedPosition < 0 ? 0 : Math.max(Math.abs(this.buffer[queuedPosition]), Math.abs(this.buffer[queuedPosition + 1]));
          const distance = ((size - queuedPosition + position) % size) / 2;
          const slope = (target - this.limit / queuedPeak) / distance;
          if (slope < this.nextSlope[index]) {
            this.nextSlope[index] = slope;
            this.queueLength = item - this.queueStart + 1;
            const tail = (this.queueStart + this.queueLength) % size;
            this.nextPosition[tail] = position;
            this.nextSlope[tail] = releaseSlope;
            this.nextPosition[(tail + 1) % size] = -1;
            this.queueLength++;
            break;
          }
        }
      }
    }
    const outputPosition = (position + 2) % size;
    const delayedLeft = this.buffer[outputPosition];
    const delayedRight = this.buffer[outputPosition + 1];
    this.gain += this.slope;
    this.left = Math.max(-this.limit, Math.min(this.limit, delayedLeft * this.gain));
    this.right = Math.max(-this.limit, Math.min(this.limit, delayedRight * this.gain));
    if (outputPosition === this.nextPosition[this.queueStart]) {
      this.slope = this.nextSlope[this.queueStart];
      this.gain = this.limit / Math.max(Math.abs(delayedLeft), Math.abs(delayedRight));
      this.queueLength--;
      this.nextPosition[this.queueStart] = -1;
      this.queueStart = (this.queueStart + 1) % size;
    }
    if (this.gain > 1) {
      this.gain = 1;
      this.slope = 0;
      this.queueStart = 0;
      this.queueLength = 0;
      this.nextPosition[0] = -1;
    }
    if (this.gain <= 0) {
      this.gain = 1e-13;
      this.slope = (1 - this.gain) / (this.rate * 0.05);
    }
    if (this.gain !== 1 && 1 - this.gain < 1e-13) this.gain = 1;
    if (this.slope !== 0 && Math.abs(this.slope) < 1e-14) this.slope = 0;
    this.position = outputPosition;
  }
}

if (typeof registerProcessor === 'function') {
  class ScreenrecordLimiter extends AudioWorkletProcessor {
    constructor() {
      super();
      this.limiter = new LookaheadLimiter(sampleRate);
      this.port.onmessage = event => { if (event.data === 'reset') this.limiter.reset(); };
    }

    process(inputs, outputs) {
      const input = inputs[0];
      const output = outputs[0];
      const left = input[0];
      const right = input[1] ?? left;
      for (let frame = 0; frame < output[0].length; frame++) {
        this.limiter.process(left?.[frame] ?? 0, right?.[frame] ?? 0);
        output[0][frame] = this.limiter.left;
        output[1][frame] = this.limiter.right;
      }
      return true;
    }
  }
  registerProcessor('screenrecord-limiter', ScreenrecordLimiter);
}
