import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from '@playwright/test'

/**
 * The traces stress run: `pnpm test:stress`. Starts the dev server on its
 * own port, drives /stress.html at several trace sizes and reads the
 * figures off the DevTools protocol. One worker, no retries — a flaky
 * performance number is a finding, not noise to retry away.
 */
export default defineConfig({
  testDir: '.',
  testMatch: 'traces-stress.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 600_000,
  expect: { timeout: 30_000 },
  reporter: [['list']],
  outputDir: '../../../../target/console-stress/playwright-output',
  use: {
    baseURL: 'http://127.0.0.1:4190',
    browserName: 'chromium',
    viewport: { width: 1440, height: 900 },
    launchOptions: {
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
        : {}),
      ...(process.env.CONSOLE_E2E_BROWSER_CHANNEL
        ? { channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL }
        : {}),
    },
    trace: 'off',
  },
  webServer: {
    command: 'pnpm exec vite --port 4190 --strictPort',
    cwd: fileURLToPath(new URL('../..', import.meta.url)),
    url: 'http://127.0.0.1:4190/stress.html?spans=10',
    reuseExistingServer: true,
    timeout: 120_000,
  },
})
