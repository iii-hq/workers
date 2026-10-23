import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    // The bundle is built once (esbuild + babel), then every test runs it in QuickJS.
    globalSetup: ['test/global-setup.ts'],
    // Relative times and dates are computed in the script: pin the zone.
    env: { TZ: 'UTC' },
    testTimeout: 30_000,
    hookTimeout: 120_000,
  },
})
