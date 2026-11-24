/**
 * What the console page opens. The page never picks a program: it asks here,
 * with the folder it wants the session in, and gets the binary, argv, folder
 * and environment of a session prepared for it NOW (`prepareSession`): the
 * `iii` plugin checked in that folder, the http worker's current address,
 * the default-mcp file written. That is what keeps the page a Claude
 * terminal rather than a shell with extra steps.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import { type HostPrepared, prepareSession } from './workspace.js';

export function registerTerminal(
  iii: IIIClient,
  host: () => HostPrepared,
  config: () => TerminalConfig,
): void {
  iii.registerFunction(
    'claude::terminal::describe',
    async (payload: unknown) => {
      const cwd =
        payload &&
        typeof payload === 'object' &&
        typeof (payload as { cwd?: unknown }).cwd === 'string'
          ? (payload as { cwd: string }).cwd
          : undefined;
      const spec = await prepareSession(iii, config(), host(), { cwd });
      if (spec.detail) console.warn(`claude-code terminal: ${spec.detail}`);
      return spec;
    },
    {
      description:
        'Prepare and describe one Claude terminal session: the claude binary, its argv, the folder it runs in (cwd, else the compose project folder), and its environment. Installs the iii Claude Code plugin in that folder when it is missing. The console page passes the result straight to shell::pty::open.',
      request_format: {
        type: 'object',
        properties: {
          cwd: {
            type: 'string',
            description:
              "Folder the session runs in, on the terminal host. Empty: the compose project's folder.",
          },
        },
      },
      response_format: {
        type: 'object',
        required: ['program', 'args', 'cwd', 'env'],
        properties: {
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
        },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );
}
