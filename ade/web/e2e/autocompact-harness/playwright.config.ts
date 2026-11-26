import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from '@playwright/test'

/* The auto-compact harness's own run (`pnpm test:e2e:autocompact`): a
   production build served by `vite preview` on 4181, no engine. */
export default defineConfig({
  testDir: '..',
  testMatch: 'autocompact-settings.spec.ts',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  expect: { timeout: 8_000 },
  reporter: [['list']],
  outputDir: '../../../../target/autocompact-e2e',
  use: {
    baseURL: 'http://127.0.0.1:4181',
    browserName: 'chromium',
    viewport: { width: 1440, height: 1000 },
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
      : undefined,
    trace: 'retain-on-failure',
  },
  webServer: {
    command:
      'pnpm exec vite build --config e2e/autocompact-harness/vite.config.ts && pnpm exec vite preview --config e2e/autocompact-harness/vite.config.ts',
    cwd: fileURLToPath(new URL('../..', import.meta.url)),
    url: 'http://127.0.0.1:4181/e2e/autocompact-harness/index.html',
    reuseExistingServer: false,
  },
})
