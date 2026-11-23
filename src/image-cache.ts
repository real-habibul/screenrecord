interface DecodedImage { naturalWidth: number; naturalHeight: number; src: string }

export class PreviewImageCache<T extends DecodedImage> {
  private images = new Map<string, T>()
  constructor(private maxBytes = 64 * 1024 * 1024, private maxCount = 8) {}
  get bytes() { return [...this.images.values()].reduce((sum, image) => sum + image.naturalWidth * image.naturalHeight * 4, 0) }
  get(key: string) { return this.images.get(key) }
  has(key: string) { return this.images.has(key) }
  put(key: string, image: T, active: string | string[]): boolean {
    const protectedKeys = typeof active === 'string' ? [active] : active
    const previous = this.images.get(key); if (previous && previous !== image) previous.src = ''
    this.images.delete(key); this.images.set(key, image)
    while (this.bytes > this.maxBytes || this.images.size > this.maxCount) {
      const removable = [...this.images.keys()].find(value => !protectedKeys.includes(value)) ?? [...protectedKeys].reverse().find(value => this.images.has(value)) ?? this.images.keys().next().value
      if (removable === undefined) break
      this.images.get(removable)!.src = ''; this.images.delete(removable)
    }
    return this.images.has(key)
  }
  clear() { this.images.forEach(image => { image.src = '' }); this.images.clear() }
}
