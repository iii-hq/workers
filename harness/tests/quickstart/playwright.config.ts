import path from 'node:path'
import { defineConfig } from '@playwright/test'

const artifactsRoot = process.env.HARNESS_QUICKSTART_ARTIFACTS_DIR
if (!artifactsRoot) {
  throw new Error('HARNESS_QUICKSTART_ARTIFACTS_DIR is required')
}

export default defineConfig({
  testDir: __dirname,
  testMatch: [
    'console-first-message.spec.ts',
    'console-first-capability.spec.ts',
    'console-router-recovery.spec.ts',
  ],
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 300_000,
  expect: { timeout: 30_000 },
  reporter: [['list']],
  outputDir: path.join(artifactsRoot, 'playwright-output'),
  use: {
    browserName: 'chromium',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'on',
  },
})
