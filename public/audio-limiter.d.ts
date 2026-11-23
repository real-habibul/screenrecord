export class LookaheadLimiter {
  constructor(rate?: number)
  readonly latencyFrames: number
  left: number
  right: number
  reset(): void
  process(left: number, right: number): void
}
