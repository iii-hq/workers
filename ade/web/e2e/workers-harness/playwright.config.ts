import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '..',
  testMatch: 'workers-settings.spec.ts',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  reporter: [['list']],
  outputDir: '../../../../target/workers-settings-e2e',
  use: {
    baseURL: 'http://127.0.0.1:4180',
    browserName: 'chromium',
    viewport: { width: 1656, height: 1314 },
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
      : undefined,
    trace: 'retain-on-failure',
  },
  webServer: {
    command:
      'pnpm exec vite build --config e2e/workers-harness/vite.config.ts && pnpm exec vite preview --config e2e/workers-harness/vite.config.ts',
    cwd: fileURLToPath(new URL('../..', import.meta.url)),
    url: 'http://127.0.0.1:4180/e2e/workers-harness/index.html',
    reuseExistingServer: true,
  },
})
