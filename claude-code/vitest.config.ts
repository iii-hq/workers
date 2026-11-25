import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
    // No test runs the real `claude plugin` commands (tests/_helpers/setup.ts).
    setupFiles: ['tests/_helpers/setup.ts'],
    environment: 'node',
    globals: false,
    testTimeout: 10000,
  },
});
