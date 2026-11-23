export type SafeAreaChoice = 'off' | 'tiktok' | 'reels'

export const safeAreas = {
  tiktok: { top: 10, right: 20, bottom: 25, left: 8 },
  reels: { top: 14, right: 16, bottom: 35, left: 6 },
} as const

export function safeAreaChoice(value: string | null): SafeAreaChoice {
  return value === 'tiktok' || value === 'reels' ? value : 'off'
}

export function supportsSafeArea(width: number, height: number) {
  return width > 0 && height > width && Math.abs(height - width * 16 / 9) <= 2
}
