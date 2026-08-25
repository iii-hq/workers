/**
 * Everything this worker does to a filesystem or a process, it does through
 * the `shell` worker — the same worker that owns the terminal session. So the
 * agent, its workspace, and its installer all live on ONE host, whether or
 * not that is the host this worker runs on, and none of it depends on a
 * shared directory.
 */

import type { IIIClient } from 'iii-sdk';

const EXEC_TIMEOUT_MS = 5 * 60_000;

export type ExecResult = { stdout: string; stderr: string; exit_code: number };

export async function exec(
  iii: IIIClient,
  command: string,
  options: { cwd?: string; timeoutMs?: number; env?: Record<string, string> } = {},
): Promise<ExecResult> {
  const res = await iii.trigger<unknown, Partial<ExecResult>>({
    function_id: 'shell::exec',
    payload: {
      command,
      ...(options.cwd ? { cwd: options.cwd } : {}),
      ...(options.env && Object.keys(options.env).length > 0 ? { env: options.env } : {}),
      timeout_ms: options.timeoutMs ?? EXEC_TIMEOUT_MS,
    },
    timeoutMs: (options.timeoutMs ?? EXEC_TIMEOUT_MS) + 30_000,
  });
  return {
    stdout: res?.stdout ?? '',
    stderr: res?.stderr ?? '',
    exit_code: res?.exit_code ?? 0,
  };
}

/** stdout of a command, or '' when it fails — for probes like `command -v`. */
export async function probe(iii: IIIClient, command: string): Promise<string> {
  try {
    const result = await exec(iii, command, { timeoutMs: 15_000 });
    return result.exit_code === 0 ? result.stdout.trim() : '';
  } catch (err) {
    console.warn(`probe failed (${command}): ${String(err)}`);
    return '';
  }
}

export async function mkdir(iii: IIIClient, path: string): Promise<void> {
  await iii.trigger({
    function_id: 'shell::fs::mkdir',
    payload: { path, parents: true },
    timeoutMs: 30_000,
  });
}

export async function writeFile(iii: IIIClient, path: string, content: string): Promise<void> {
  await iii.trigger({
    function_id: 'shell::fs::write',
    payload: { path, content, parents: true },
    timeoutMs: 60_000,
  });
}

/**
 * File contents, or null when the file is absent. `coder::read-file` returns
 * the text inline; `shell::fs::read` streams through a channel, which a
 * worker reading a small settings file does not need.
 */
export async function readFile(iii: IIIClient, path: string): Promise<string | null> {
  try {
    const res = await iii.trigger<unknown, { content?: string }>({
      function_id: 'coder::read-file',
      payload: { path },
      timeoutMs: 30_000,
    });
    return typeof res?.content === 'string' ? res.content : null;
  } catch {
    return null;
  }
}

/** The terminal host's working directory — the shell worker's primary root. */
export async function hostRoot(iii: IIIClient): Promise<string> {
  const pwd = await probe(iii, 'pwd');
  return pwd || '.';
}
