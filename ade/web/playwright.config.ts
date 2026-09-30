import path from 'node:path'
import { defineConfig } from '@playwright/test'
import { genericConsoleTestIgnore } from './e2e/namespaced-provider-selection'

const artifactsRoot =
  process.env.CONSOLE_E2E_ARTIFACTS_DIR ??
  path.resolve(import.meta.dirname, '../../target/console-e2e')

export default defineConfig({
  testDir: './e2e',
  testIgnore: genericConsoleTestIgnore(),
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  outputDir: path.join(artifactsRoot, 'playwright-output'),
  use: {
    browserName: 'chromium',
    launchOptions: {
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
        : {}),
      ...(process.env.CONSOLE_E2E_BROWSER_CHANNEL
        ? { channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL }
        : {}),
    },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
})
