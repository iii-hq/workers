/**
 * What the console page opens. The page never picks a program: it asks here,
 * with the folder it wants the session in (or a listed session to open), and
 * gets the binary, argv, folder and environment of a session prepared for it
 * NOW (`prepareSession`): the `iii` plugin checked in that folder, the http
 * worker's current address, the default-mcp file written. That is what keeps
 * the page a Claude terminal rather than a shell with extra steps.
 */

import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import type { OpenSpec, ResolveOpen } from '../agents.js';
import type { TerminalConfig } from '../config.js';
import { jsonSchema } from '../run.js';
import { type HostPrepared, prepareSession } from './workspace.js';

const DescribePayloadSchema = z.object({
  reopen_held_resume: z.boolean().optional().describe('Obsolete; lease-only reopen is refused.'),
  mode: z
    .enum(['original', 'fork'])
    .optional()
    .describe('Explicit session action. Without a mode, preserve the legacy automatic behavior.'),
  claude_session_id: z
    .guid()
    .optional()
    .describe('Expected conversation UUID from the selected row; required with mode.'),
  cwd: z
    .string()
    .max(4096)
    .optional()
    .describe(
      "Folder the session runs in, on the terminal host. Empty: the compose project's folder. With `session`, the session's own folder wins.",
    ),
  session: z
    .string()
    .min(1)
    .max(200)
    .optional()
    .describe('The session to open: a row id of claude::agents::list. Omitted = a new session.'),
});

/**
 * The argv that opens a listed session. `resolveOpen` answers it relative to
 * the session's own prepared argv (the inventory host carries none): a resume
 * is appended to it, so the session keeps default-mcp and the operator's
 * args; `attach` joins a running background session and takes nothing else.
 */
export function openArgs(prepared: string[], open: OpenSpec): string[] {
  return open.args[0] === 'attach' ? open.args : [...prepared, ...open.args];
}

export function registerTerminal(
  iii: IIIClient,
  host: () => HostPrepared,
  config: () => TerminalConfig,
  resolveOpen: ResolveOpen,
): void {
  iii.registerFunction(
    'claude::terminal::describe',
    async (payload: unknown) => {
      const { cwd, session, mode, claude_session_id, reopen_held_resume } =
        DescribePayloadSchema.parse(payload ?? {});
      const prepared = host();
      if (!prepared.executable) {
        throw new Error(prepared.detail || 'claude is not available on the terminal host');
      }
      if (mode && (!session || !claude_session_id))
        throw new Error('an explicit action needs a session and conversation UUID');
      if (reopen_held_resume)
        throw new Error(
          'a saved terminal cannot verify the original conversation; refresh the list',
        );
      const request = mode && claude_session_id ? { mode, claude_session_id } : undefined;
      const open = session
        ? request
          ? await resolveOpen(session, request, false)
          : await resolveOpen(session)
        : undefined;
      // A viewer or an existing PTY needs no new process, plugin or workspace setup.
      if (open && !('args' in open)) return open;
      const spec = await prepareSession(iii, config(), prepared, { cwd: open?.cwd || cwd });
      if (spec.detail) console.warn(`claude-code terminal: ${spec.detail}`);
      if (!open) return spec;
      // Plugin preparation can take minutes. Never give out an earlier attach,
      // resume or copy if the inventory now calls for something different.
      if (session && request) {
        const samePlan = (fresh: OpenSpec) =>
          fresh.cwd === open.cwd &&
          fresh.copy === open.copy &&
          fresh.args.length === open.args.length &&
          fresh.args.every((arg, i) => arg === open.args[i]);
        const fresh = await resolveOpen(session, request, samePlan);
        if (!('args' in fresh) || !samePlan(fresh)) {
          throw new Error('the session changed while opening; refresh the list');
        }
      }
      return {
        ...spec,
        args: openArgs(spec.args, open),
        ...(request ? { claude_session_id: request.claude_session_id } : {}),
        ...(open.copy ? { copy: true } : {}),
      };
    },
    {
      description:
        'Prepare and describe one Claude terminal session: the claude binary, its argv, the folder it runs in (cwd, else the compose project folder), and its environment. Installs the iii Claude Code plugin in that folder when it is missing. With `session`, the argv and folder that open that listed session, or an error that says why it cannot be opened. An explicit original action can instead return an existing PTY or a read-only follow target. An explicit fork always creates a separate conversation.',
      request_format: jsonSchema(DescribePayloadSchema),
      response_format: {
        type: 'object',
        anyOf: [
          { required: ['program', 'args', 'cwd', 'env'] },
          { required: ['pty_session_id', 'claude_session_id'] },
          { required: ['follow', 'claude_session_id'] },
        ],
        properties: {
          pty_session_id: {
            type: 'string',
            description: 'Reopen this identified original console PTY; do not launch a program.',
          },
          follow: {
            const: true,
            description: 'No original transport: use the read-only reply APIs.',
          },
          claude_session_id: { type: 'string' },
          program: { type: 'string' },
          args: { type: 'array', items: { type: 'string' } },
          cwd: { type: 'string', description: 'The folder, resolved (symlinks followed).' },
          env: { type: 'object', additionalProperties: { type: 'string' } },
          activity_bridge: {
            type: 'string',
            description:
              "The `iii` CLI on the terminal host that carries this session's activity to the bus (III_CLI). Empty means the hooks use `iii` on PATH.",
          },
          detail: {
            type: 'string',
            description: 'Anything wrong that does not stop a session from opening.',
          },
          plugin: {
            type: ['object', 'null'],
            description:
              'The iii plugin check in cwd: state present, installed, enabled or failed; null when setup_workspace is off.',
            properties: {
              state: { type: 'string' },
              folder: { type: 'string' },
              detail: { type: 'string' },
            },
          },
          project_dir: {
            type: 'string',
            description: "The compose project's folder, the default when no cwd is given.",
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
