import type { ChildProcess } from 'node:child_process';
import { isAbsolute, resolve } from 'node:path';

export type Instance = { id: string; name: string; workspace: string; port: number; url: string; pid: number | null; started_at: string; status: 'starting'|'running'|'stopped'|'failed'; exit_code?: number|null; process: ChildProcess };
export const idPattern = /^[a-z0-9][a-z0-9-]{0,47}$/;
export function schema(properties: Record<string, unknown>, required: string[] = []) { return { type: 'object' as const, properties, required }; }
export function validateId(id: string) { if (!idPattern.test(id)) throw new Error('id must match ^[a-z0-9][a-z0-9-]{0,47}$'); return id; }
export function validateWorkspacePath(path: string) { if (!isAbsolute(path)) throw new Error('workspace must be an absolute path'); return resolve(path); }
export function serverArgs(input: { bindHost: string; port: number; serverData: string; cliData: string; workspace: string }) {
  if (input.bindHost !== '127.0.0.1' && input.bindHost !== 'localhost' && input.bindHost !== '::1') {
    throw new Error('cookie-free VS Code Server must bind to loopback');
  }
  return ['--cli-data-dir', input.cliData, 'serve-web', '--host', input.bindHost, '--port', String(input.port), '--without-connection-token', '--accept-server-license-terms', '--server-data-dir', input.serverData, '--default-folder', input.workspace, '--disable-telemetry'];
}
export function publicInstance(instance: Instance) { return { id: instance.id, name: instance.name, workspace: instance.workspace, port: instance.port, pid: instance.pid, started_at: instance.started_at, status: instance.status, exit_code: instance.exit_code ?? null }; }
