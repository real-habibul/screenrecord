export class EditHistory {
  private past: string[] = []
  private future: string[] = []
  private present = ''
  private pending: string | null = null
  constructor(private capacity = 100, private byteBudget = 16 * 1024 * 1024) {}
  reset(snapshot: string) { this.past = []; this.future = []; this.present = snapshot; this.pending = null }
  observe(snapshot: string) { this.pending = snapshot === this.present ? null : snapshot }
  commit() {
    if (this.pending === null) return
    this.past.push(this.present)
    this.present = this.pending; this.pending = null; this.future = []
    this.prune()
  }
  private prune() {
    let bytes = [...this.past, ...this.future].reduce((total, value) => total + value.length * 2, 0)
    while ((this.past.length + this.future.length > this.capacity || bytes > this.byteBudget) && (this.past.length || this.future.length)) {
      const removed = this.past.length ? this.past.shift()! : this.future.shift()!
      bytes -= removed.length * 2
    }
  }
  get canUndo() { return this.pending !== null || this.past.length > 0 }
  get canRedo() { return this.pending === null && this.future.length > 0 }
  undo(): string | null {
    this.commit()
    const snapshot = this.past.pop()
    if (snapshot === undefined) return null
    this.future.push(this.present); this.present = snapshot; this.prune()
    return snapshot
  }
  redo(): string | null {
    if (this.pending !== null) return null
    const snapshot = this.future.pop()
    if (snapshot === undefined) return null
    this.past.push(this.present); this.present = snapshot; this.prune()
    return snapshot
  }
}
