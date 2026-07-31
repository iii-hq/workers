// Git access for openwiki. Routes through the `shell` worker (`shell::exec`) so
// cloning, HEAD reads, and diffs run on the iii bus like everything else. Falls
// back to a local child_process when the shell worker is absent, so the worker
// still runs standalone.
//
// worktree deployment note: the `shell` worker jails exec/fs under
// `fs.host_roots`. The clone directory (OPENWIKI_DATA/repos/<id>, default
// /tmp/openwiki-data) MUST resolve inside those roots, or shell::exec refuses
// the clone (S215). Point OPENWIKI_DATA inside host_roots, or run without the
// shell worker to use the local-git fallback.
import { spawn } from 'node:child_process';

function runLocal(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { ...opts });
    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', (d) => {
      stdout += d.toString();
    });
    child.stderr?.on('data', (d) => {
      stderr += d.toString();
    });
    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) resolve({ stdout, stderr });
      else reject(new Error(`${cmd} ${args.join(' ')} exited ${code}: ${stderr.trim()}`));
    });
  });
}

// Run a command via shell::exec; fall back to local child_process when the
// shell worker is unreachable. A non-zero exit is surfaced (never falls back —
// that would double-run side effects like a clone).
async function sh(worker, command, args, { cwd, timeoutMs } = {}) {
  if (worker) {
    let res = null;
    try {
      res = await worker.trigger({
        function_id: 'shell::exec',
        payload: {
          command,
          args,
          ...(cwd ? { cwd } : {}),
          ...(timeoutMs ? { timeout_ms: timeoutMs } : {}),
        },
        timeoutMs: (timeoutMs || 120_000) + 10_000,
      });
    } catch {
      res = null; // shell worker unreachable -> local fallback
    }
    if (res != null) {
      // Reached the worker. Never fall back from here: a fallback could
      // double-run side effects (a second clone). Surface the outcome.
      if (typeof res.exit_code === 'number') {
        if (res.exit_code === 0) return { stdout: res.stdout || '', stderr: res.stderr || '' };
        throw new Error(`${command} ${args.join(' ')} exited ${res.exit_code}: ${String(res.stderr || '').trim()}`);
      }
      throw new Error(`${command} ${args.join(' ')}: shell::exec returned no exit_code`);
    }
  }
  return runLocal(command, args, { cwd });
}

export function repoName(repoUrl) {
  const u = String(repoUrl)
    .trim()
    .replace(/\.git$/, '')
    .replace(/\/+$/, '');
  const parts = u.split(/[/:]/).filter(Boolean);
  if (parts.length >= 2) return parts.slice(-2).join('/');
  return parts[parts.length - 1] || u;
}

export async function cloneRepo(worker, repoUrl, destDir, ref) {
  const args = ['clone', '--depth', '1'];
  if (ref) args.push('--branch', ref);
  args.push(repoUrl, destDir);
  await sh(worker, 'git', args, { timeoutMs: 120_000 });
  const { stdout } = await sh(worker, 'git', ['-C', destDir, 'rev-parse', 'HEAD']);
  return { commit: stdout.trim(), dir: destDir, name: repoName(repoUrl), url: repoUrl };
}

export async function currentCommit(worker, dir) {
  const { stdout } = await sh(worker, 'git', ['-C', dir, 'rev-parse', 'HEAD']);
  return stdout.trim();
}

// Best-effort update of an existing clone. Returns the new HEAD, or null on
// failure (caller decides whether to re-clone).
export async function gitPull(worker, dir) {
  try {
    await sh(worker, 'git', ['-C', dir, 'fetch', '--depth', '1', 'origin'], { timeoutMs: 120_000 });
    await sh(worker, 'git', ['-C', dir, 'reset', '--hard', 'FETCH_HEAD']);
    return await currentCommit(worker, dir);
  } catch {
    return null;
  }
}

export async function gitDiff(worker, dir, baseRef, headRef = 'HEAD') {
  const { stdout } = await sh(worker, 'git', ['-C', dir, 'diff', '--name-status', `${baseRef}..${headRef}`]);
  return parseNameStatus(stdout);
}

export function parseNameStatus(stdout) {
  const out = [];
  for (const line of String(stdout).split('\n')) {
    if (!line.trim()) continue;
    const parts = line.split(/\t/);
    const status = parts[0][0];
    if (!'AMDR'.includes(status)) continue;
    const p = status === 'R' ? parts[2] : parts[1];
    if (p) out.push({ status, path: p });
  }
  return out;
}
