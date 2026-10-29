import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    // Each file builds its fixture bundles (esbuild + babel) before running them in QuickJS.
    testTimeout: 60_000,
    hookTimeout: 120_000,
  },
})
