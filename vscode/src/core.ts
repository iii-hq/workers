import type { ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { isAbsolute, resolve } from 'node:path';

export type InstanceStatus = 'starting' | 'running' | 'stopped' | 'failed';

export type Instance = {
  id: string;
  name: string;
  workspace: string;
  host: string;
  port: number;
  url: string;
  pid: number | null;
  started_at: string;
  status: InstanceStatus;
  exit_code?: number | null;
  process: ChildProcess;
};

export type PublicInstance = Omit<Instance, 'url' | 'process' | 'exit_code'> & {
  exit_code: number | null;
};

export const idPattern = /^[a-z0-9][a-z0-9-]{0,47}$/;

const loopbackHosts = new Set(['127.0.0.1', 'localhost', '::1']);

export function schema(properties: Record<string, unknown>, required: string[] = []) {
  return { type: 'object' as const, properties, required };
}

export function validateId(id: string) {
  if (typeof id !== 'string' || !idPattern.test(id)) {
    throw new Error('id must match ^[a-z0-9][a-z0-9-]{0,47}$');
  }
  return id;
}

export function validateWorkspacePath(path: unknown) {
  if (typeof path !== 'string' || !isAbsolute(path)) {
    throw new Error('workspace must be an absolute path');
  }
  return resolve(path);
}

export function instanceIdFor(workspace: string) {
  const digest = createHash('sha256').update(workspace).digest('hex').slice(0, 12);
  return `ide-${digest}`;
}

export function isLoopback(host: string) {
  return loopbackHosts.has(host);
}

export function serverArgs(input: {
  bindHost: string;
  port: number;
  serverData: string;
  cliData: string;
  workspace: string;
}) {
  if (!isLoopback(input.bindHost)) {
    throw new Error('cookie-free VS Code Server must bind to loopback');
  }
  return [
    '--cli-data-dir',
    input.cliData,
    'serve-web',
    '--host',
    input.bindHost,
    '--port',
    String(input.port),
    '--without-connection-token',
    '--accept-server-license-terms',
    '--server-data-dir',
    input.serverData,
    '--default-folder',
    input.workspace,
    '--disable-telemetry',
  ];
}

export function publicInstance(instance: Instance): PublicInstance {
  return {
    id: instance.id,
    name: instance.name,
    workspace: instance.workspace,
    host: instance.host,
    port: instance.port,
    pid: instance.pid,
    started_at: instance.started_at,
    status: instance.status,
    exit_code: instance.exit_code ?? null,
  };
}
