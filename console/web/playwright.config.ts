import path from 'node:path'
import { defineConfig } from '@playwright/test'

const artifactsRoot =
  process.env.CONSOLE_E2E_ARTIFACTS_DIR ??
  path.resolve(import.meta.dirname, '../../target/console-e2e')

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  outputDir: path.join(artifactsRoot, 'playwright-output'),
  use: {
    browserName: 'chromium',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
})
