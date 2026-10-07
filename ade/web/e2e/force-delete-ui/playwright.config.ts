import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  defineConfig,
  type PlaywrightTestConfig,
  type Project,
} from '@playwright/test'

const root = path.dirname(fileURLToPath(import.meta.url))
const port = Number(process.env.FORCE_DELETE_UI_PORT ?? 4179)
const baseURL = `http://127.0.0.1:${port}`

// Shared by the standalone matrix and the generic CI invocation. Native
// Console specs retain their per-test real stack; only this project uses Vite.
export const forceDeleteFixtureProject: Project = {
  name: 'force-delete-fixture',
  testDir: root,
  testMatch: 'force-delete.spec.ts',
  timeout: 30_000,
  use: { baseURL },
}
export const forceDeleteFixtureServer: NonNullable<
  PlaywrightTestConfig['webServer']
> = {
  command: `pnpm exec vite --config e2e/force-delete-ui/vite.config.ts --port ${port}`,
  cwd: path.resolve(root, '../..'),
  url: baseURL,
  reuseExistingServer: false,
}
export default defineConfig({
  projects: [forceDeleteFixtureProject],
  workers: 1,
  outputDir: path.resolve(root, '../../../../target/force-delete-ui'),
  use: {
    browserName: 'chromium',
    launchOptions: {
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
        ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH }
        : {}),
    },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: forceDeleteFixtureServer,
})
