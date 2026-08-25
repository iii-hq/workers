/**
 * What the console page opens. The page never picks a program: it asks here,
 * and gets the binary, argv, and workspace this worker prepared. That is what
 * keeps the page a Claude terminal rather than a shell with extra steps.
 */

import type { IIIClient } from 'iii-sdk';
import type { Prepared } from './workspace.js';

export function registerTerminal(iii: IIIClient, current: () => Prepared): void {
  iii.registerFunction(
    'claude-cli::terminal::describe',
    async () => {
      const prepared = current();
      if (!prepared.executable) {
        throw new Error(prepared.detail || 'claude is not available on the terminal host');
      }
      return {
        program: prepared.executable,
        args: prepared.args,
        cwd: prepared.workspace,
        env: prepared.env,
      };
    },
    {
      description:
        'What a Claude terminal session runs: the claude binary, its argv, the workspace directory, and the session environment. The console page passes this straight to shell::pty::open.',
      request_format: { type: 'object', properties: {} },
      response_format: {
        type: 'object',
        required: ['program', 'args', 'cwd', 'env'],
        properties: {
          program: { type: 'string' },
          args: { type: 'array', items: { type: 'string' } },
          cwd: { type: 'string' },
          env: { type: 'object', additionalProperties: { type: 'string' } },
        },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );
}
