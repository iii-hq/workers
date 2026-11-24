/**
 * What the console page opens. The page never picks a program: it asks here,
 * and gets the binary, argv, and workspace this worker prepared. That is what
 * keeps the page a Claude terminal rather than a shell with extra steps.
 */

import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import type { OpenSpec } from '../agents.js';
import { jsonSchema } from '../run.js';
import type { Prepared } from './workspace.js';

const DescribePayloadSchema = z.object({
  session: z
    .string()
    .min(1)
    .max(200)
    .optional()
    .describe('The session to open: a row id of claude::agents::list. Omitted = a new session.'),
});

export function registerTerminal(
  iii: IIIClient,
  current: () => Prepared,
  resolveOpen: (id: string) => Promise<OpenSpec>,
): void {
  iii.registerFunction(
    'claude::terminal::describe',
    async (payload: unknown) => {
      const { session } = DescribePayloadSchema.parse(payload ?? {});
      const prepared = current();
      if (!prepared.executable) {
        throw new Error(prepared.detail || 'claude is not available on the terminal host');
      }
      return {
        program: prepared.executable,
        args: prepared.args,
        cwd: prepared.workspace,
        env: prepared.env,
        activity_bridge: prepared.bridge,
        detail: prepared.detail,
        ...(session ? await resolveOpen(session) : {}),
      };
    },
    {
      description:
        'What a Claude terminal session runs: the claude binary, its argv, the workspace directory, and the session environment. The console page passes this straight to shell::pty::open. With `session`, the argv and directory that open that listed session, or an error that says why it cannot be opened.',
      request_format: jsonSchema(DescribePayloadSchema),
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
              "The `iii` CLI on the terminal host that carries this session's activity to the bus. Empty means nothing will reach the claude::on-agent-event feed.",
          },
          detail: {
            type: 'string',
            description: 'Anything wrong that does not stop a session from opening.',
          },
          copy: {
            type: 'boolean',
            description:
              'True when the asked session is running elsewhere, so this opens a copy of its conversation under a new id.',
          },
        },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );
}
