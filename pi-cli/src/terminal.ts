/**
 * What the console page opens. The page never picks a program: it asks here,
 * and gets the binary, argv, and workspace this worker prepared. That is what
 * keeps the page a Pi terminal rather than a shell with extra steps.
 */

import type { IIIClient } from 'iii-sdk';
import type { Prepared } from './workspace.js';

export function registerTerminal(iii: IIIClient, current: () => Prepared): void {
  iii.registerFunction(
    'pi-cli::terminal::describe',
    async () => {
      const prepared = current();
      if (!prepared.executable) {
        throw new Error(prepared.detail || 'pi is not available on the terminal host');
      }
      return {
        program: prepared.executable,
        args: prepared.args,
        cwd: prepared.workspace,
        env: prepared.env,
        activity_bridge: prepared.bridge,
        detail: prepared.detail,
      };
    },
    {
      description:
        'What a Pi terminal session runs: the pi binary, its argv, the workspace directory, and the session environment. The console page passes this straight to shell::pty::open.',
      request_format: { type: 'object', properties: {} },
      response_format: {
        type: 'object',
        required: ['program', 'args', 'cwd', 'env'],
        properties: {
          program: { type: 'string' },
          args: { type: 'array', items: { type: 'string' } },
          cwd: { type: 'string' },
          env: { type: 'object', additionalProperties: { type: 'string' } },
          activity_bridge: {
            type: 'string',
            description:
              "The `iii` CLI on the terminal host that carries this session's activity to the bus. Empty means nothing will reach the events stream.",
          },
          detail: {
            type: 'string',
            description: 'Anything wrong that does not stop a session from opening.',
          },
        },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );
}
