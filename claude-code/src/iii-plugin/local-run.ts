/**
 * Run a program on THIS worker's host, the one headless `claude::run` turns
 * run on: argv only (no shell), in `cwd`, never throwing. The terminal half
 * runs the same commands through `shell::exec` on the terminal host instead
 * (`../terminal/workspace.ts`). A module of its own so tests replace it
 * (`tests/_helpers/setup.ts`): no test may run the real `claude plugin`.
 */

import { execFile } from 'node:child_process';
import type { Runner } from './install.js';

const TIMEOUT_MS = 5 * 60_000;

export const localRun: Runner = (program, args, cwd) =>
  new Promise((resolve) => {
    execFile(
      program,
      args,
      { cwd, timeout: TIMEOUT_MS, maxBuffer: 16 * 1024 * 1024, encoding: 'utf8' },
      (error, stdout, stderr) => {
        if (!error) return resolve({ stdout, stderr, exit_code: 0 });
        const code = (error as NodeJS.ErrnoException & { code?: unknown }).code;
        resolve({
          stdout: stdout ?? '',
          stderr: stderr || error.message,
          // A spawn failure (no such program, no such folder) has a string code.
          exit_code: typeof code === 'number' ? code : 127,
        });
      },
    );
  });
