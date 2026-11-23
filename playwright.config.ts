import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests/ui',
  timeout: 30_000,
  fullyParallel: false,
  use: {
    baseURL: 'http://127.0.0.1:1420',
    viewport: { width: 1440, height: 900 },
    launchOptions: { executablePath: process.env.BROWSER_PATH ?? 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe' },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: { command: 'npm run dev', url: 'http://127.0.0.1:1420', reuseExistingServer: true },
})
