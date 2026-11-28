/**
 * What the console page opens. The page never picks a program: it asks here,
 * with the folder it wants the session in (or a listed session to open), and
 * gets the binary, argv, folder and environment of a session prepared for it
 * NOW (`prepareSession`): the `iii` plugin checked in that folder, the http
 * worker's current address, the default-mcp file written. That is what keeps
 * the page a Claude terminal rather than a shell with extra steps.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import type { OpenRequest, OpenSpec, OpenVisibility, ResolveOpen } from '../agents.js';
import type { TerminalConfig } from '../config.js';
import { jsonSchema } from '../run.js';
import { type HostPrepared, prepareSession } from './workspace.js';

const DescribePayloadSchema = z.object({
  mode: z
    .enum(['fork', 'resume'])
    .optional()
    .describe(
      'fork: open a separate copy. resume: reopen the original, only when no other process has it. Without a mode, preserve the legacy automatic behavior.',
    ),
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

/** `--resume <id>` at the end, without `--fork-session`, or `attach <short id>`. */
function isResumePlan(open: OpenSpec, claudeSessionId: string | undefined): boolean {
  if (open.args[0] === 'attach') return open.args.length === 2;
  return (
    open.args.at(-2) === '--resume' &&
    open.args.at(-1) === claudeSessionId &&
    !open.args.includes('--fork-session')
  );
}

export function registerTerminal(
  iii: IIIClient,
  host: () => HostPrepared,
  config: () => TerminalConfig,
  resolveOpen: ResolveOpen,
  visibilityEpoch: OpenVisibility,
): void {
  iii.registerFunction(
    'claude::terminal::describe',
    async (payload: unknown) => {
      const { cwd, session, mode, claude_session_id } = DescribePayloadSchema.parse(payload ?? {});
      const prepared = host();
      if (!prepared.executable) {
        throw new Error(prepared.detail || 'claude is not available on the terminal host');
      }
      if (mode && (!session || !claude_session_id))
        throw new Error('an explicit action needs a session and conversation UUID');
      const request: OpenRequest | undefined = !claude_session_id
        ? undefined
        : mode === 'resume'
          ? // One guard per call: its own revalidation passes the opening guard it set.
            { mode, claude_session_id, guard: randomUUID() }
          : mode === 'fork'
            ? { mode, claude_session_id }
            : undefined;
      let held: OpenSpec | undefined;
      const epoch = session ? await visibilityEpoch(mode ?? 'legacy') : null;
      const verifyVisibility = () => {
        if (session && epoch !== visibilityEpoch()) {
          throw new Error('session visibility changed; refresh the list');
        }
      };
      // A spec built for a replaced binary or workspace would open the session on a stale host.
      const verifyHost = () => {
        if (session && host() !== prepared) {
          throw new Error('the terminal host changed while opening; refresh the list');
        }
      };
      try {
        verifyVisibility();
        const open = session
          ? request
            ? await resolveOpen(session, request)
            : await resolveOpen(session)
          : undefined;
        held = open;
        if (mode === 'fork' && !open?.copy) throw new Error('no separate fork plan');
        // Resume never hands out a copy: only the original, by --resume or attach.
        if (mode === 'resume' && (!open || open.copy || !isResumePlan(open, claude_session_id))) {
          throw new Error('no resume plan for that conversation');
        }
        verifyVisibility();
        const spec = await prepareSession(iii, config(), prepared, { cwd: open?.cwd || cwd });
        verifyHost();
        // Plugin preparation can take minutes. Revalidate the explicit fork before returning it.
        if (session && request && open) {
          const fresh = await resolveOpen(session, request);
          held = fresh;
          if (
            fresh.cwd !== open.cwd ||
            fresh.copy !== open.copy ||
            fresh.args.length !== open.args.length ||
            !fresh.args.every((arg, i) => arg === open.args[i])
          ) {
            throw new Error('the session changed while opening; refresh the list');
          }
        }
        verifyHost();
        verifyVisibility();
        if (spec.detail) console.warn(`claude-code terminal: ${spec.detail}`);
        if (!open) return spec;
        return {
          ...spec,
          args: openArgs(spec.args, open),
          ...(request ? { claude_session_id: request.claude_session_id } : {}),
          ...(open.copy ? { copy: true } : {}),
        };
      } catch (error) {
        // A resume that hands out nothing must not keep its conversation held.
        held?.release?.();
        // Preparation errors can contain the external directory too, not just a successful spec.
        verifyVisibility();
        verifyHost();
        throw error;
      }
    },
    {
      description:
        'Prepare and describe one Claude terminal session: the claude binary, its argv, the folder it runs in (cwd, else the compose project folder), and its environment. Installs the iii Claude Code plugin in that folder when it is missing. With `session`, the argv and folder that open that listed session, or an error that says why it cannot be opened. An explicit fork always creates a separate conversation; an explicit resume reopens the original (`--resume`, or `attach` for a running background session) and is refused while another process has it.',
      request_format: jsonSchema(DescribePayloadSchema),
      response_format: {
        type: 'object',
        required: ['program', 'args', 'cwd', 'env'],
        properties: {
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
