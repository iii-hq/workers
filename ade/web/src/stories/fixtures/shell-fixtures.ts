import type { FunctionTriggerMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()
const JOB = 'job-9f8e7d6c-5b4a-4310-aedc-ba9876543210'
const SANDBOX = 'a1b2c3d4-5e6f-4a8b-9c0d-e1f2a3b4c5d6'

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    output,
    durationMs: 240,
    createdAt: now,
    ...extra,
  }
}

export const shellExecDone = base(
  'sh-exec',
  'shell::exec',
  {
    command: 'ls -la',
    cwd: '/srv/app',
    timeout_ms: 30_000,
  },
  wrapHarness({
    exit_code: 0,
    stdout:
      'total 8\ndrwxr-xr-x 2 app app 4096 May 26 10:00 .\ndrwxr-xr-x 1 app app 4096 May 26 09:58 ..\n-rw-r--r-- 1 app app   12 May 26 10:00 README.md\n',
    stderr: '',
    duration_ms: 142,
    timed_out: false,
    stdout_truncated: false,
    stderr_truncated: false,
  }),
)

export const shellExecArgvDone = base(
  'sh-exec-argv',
  'shell::exec',
  {
    command: 'git',
    args: ['log', '--oneline', '-3'],
    cwd: '/srv/app',
    env: { GIT_PAGER: 'cat' },
  },
  {
    exit_code: 0,
    stdout:
      '8fbe7a1 feat(console): friendly chat views\nab75f28 chore(coder): bump to v0.5.1\n82e1c5d chore(shell): bump to v0.5.0\n',
    stderr: '',
    duration_ms: 58,
    timed_out: false,
    stdout_truncated: false,
    stderr_truncated: false,
  },
)

export const shellExecSandboxDone = base(
  'sh-exec-sandbox',
  'shell::exec',
  {
    command: 'uname -a',
    target: { kind: 'sandbox', sandbox_id: SANDBOX },
  },
  wrapHarness({
    exit_code: 0,
    stdout: 'Linux sandbox 6.1.0 #1 SMP x86_64 GNU/Linux\n',
    stderr: '',
    duration_ms: 35,
    timed_out: false,
    stdout_truncated: false,
    stderr_truncated: false,
  }),
)

export const shellExecRunning = base(
  'sh-exec-running',
  'shell::exec',
  { command: 'npm test', cwd: '/srv/app' },
  undefined,
  { running: true },
)

export const shellExecPending = base(
  'sh-exec-pending',
  'shell::exec',
  { command: 'rm -rf /tmp/scratch' },
  undefined,
  { pendingApproval: true },
)

export const shellExecBgDone = base(
  'sh-exec-bg',
  'shell::exec_bg',
  { command: 'npm run build', cwd: '/srv/app' },
  wrapHarness({ job_id: JOB, argv: ['npm', 'run', 'build'] }),
)

export const shellExecBgPending = base(
  'sh-exec-bg-pending',
  'shell::exec_bg',
  { command: 'cargo build --release', cwd: '/srv/app' },
  undefined,
  { pendingApproval: true },
)

export const shellStatusFinishedDone = base(
  'sh-status-finished',
  'shell::status',
  { job_id: JOB },
  {
    job: {
      id: JOB,
      argv: ['npm', 'run', 'build'],
      started_at_ms: now - 9_500,
      finished_at_ms: now - 1_200,
      status: 'finished',
      exit_code: 0,
      stdout: '> build\n> vite build\n\ndist/index.html  0.46 kB\n',
      stderr: '',
      stdout_truncated: false,
      stderr_truncated: false,
    },
  },
)

export const shellStatusRunningDone = base(
  'sh-status-running',
  'shell::status',
  { job_id: JOB },
  wrapHarness({
    job: {
      id: JOB,
      argv: ['npm', 'run', 'build'],
      started_at_ms: now - 3_000,
      finished_at_ms: null,
      status: 'running',
      exit_code: null,
      stdout: '> build\n> vite build\n',
      stderr: '',
      stdout_truncated: false,
      stderr_truncated: false,
    },
  }),
)

export const shellKillDone = base(
  'sh-kill',
  'shell::kill',
  { job_id: JOB },
  wrapHarness({ job_id: JOB, killed: true, status: 'killed' }),
)

export const shellKillAlreadyFinished = base(
  'sh-kill-finished',
  'shell::kill',
  { job_id: JOB },
  {
    job_id: JOB,
    killed: false,
    status: 'finished',
    reason: 'job already finished',
  },
)

export const shellKillPending = base(
  'sh-kill-pending',
  'shell::kill',
  { job_id: JOB },
  undefined,
  { pendingApproval: true },
)

export const shellListDone = base(
  'sh-list',
  'shell::list',
  {},
  wrapHarness({
    jobs: [
      {
        id: JOB,
        status: 'running',
        started_at_ms: now - 3_000,
        finished_at_ms: null,
        exit_code: null,
        stdout_truncated: false,
        stderr_truncated: false,
      },
      {
        id: 'job-1a2b3c4d-0001-4000-8000-000000000001',
        status: 'finished',
        started_at_ms: now - 60_000,
        finished_at_ms: now - 58_000,
        exit_code: 0,
        stdout_truncated: false,
        stderr_truncated: false,
      },
      {
        id: 'job-1a2b3c4d-0002-4000-8000-000000000002',
        status: 'failed',
        started_at_ms: now - 120_000,
        finished_at_ms: now - 119_000,
        exit_code: 1,
        stdout_truncated: true,
        stderr_truncated: false,
      },
    ],
    count: 3,
  }),
)

export const shellConfigStatusDone = base(
  'sh-config-status',
  'shell::config-status',
  {},
  { last_outcome: 'applied', last_error: null, rejected_reloads: 0 },
)

export const shellConfigStatusRejected = base(
  'sh-config-status-rejected',
  'shell::config-status',
  {},
  wrapHarness({
    last_outcome: 'rejected',
    last_error: 'jail_dir must be an absolute path',
    rejected_reloads: 2,
  }),
)

export const shellFsLsDone = base(
  'sh-ls',
  'shell::fs::ls',
  { path: '/srv/app' },
  {
    entries: [
      {
        name: 'src',
        is_dir: true,
        size: 4096,
        mode: '0755',
        mtime: Math.floor(now / 1000) - 180,
        is_symlink: false,
      },
      {
        name: 'package.json',
        is_dir: false,
        size: 412,
        mode: '0644',
        mtime: Math.floor(now / 1000) - 60,
        is_symlink: false,
      },
    ],
  },
)

export const shellFsStatDone = base(
  'sh-stat',
  'shell::fs::stat',
  { path: '/srv/app/package.json' },
  // StatResponse is #[serde(transparent)] — a bare FsEntry, no wrapper key.
  wrapHarness({
    name: 'package.json',
    is_dir: false,
    size: 412,
    mode: '0644',
    mtime: Math.floor(now / 1000) - 60,
    is_symlink: false,
  }),
)

export const shellFsReadDone = base(
  'sh-read',
  'shell::fs::read',
  { path: '/srv/app/src/index.ts' },
  // ReadResponseWire: content is ALWAYS a streaming channel ref.
  {
    content: {
      channel_id: 'chan-2f9c1b7e-aa00-4f00-9d00-7c6b5a493827',
      access_key: 'ak_4f8e2d1c9b7a',
      direction: 'read',
    },
    size: 48,
    mode: '0644',
    mtime: Math.floor(now / 1000) - 30,
  },
)

export const shellFsWriteDone = base(
  'sh-write',
  'shell::fs::write',
  { path: '/srv/app/out.txt', content: 'hello\n', parents: true },
  wrapHarness({ bytes_written: 6, path: '/srv/app/out.txt', files: [] }),
)

export const shellFsWriteBatchDone = base(
  'sh-write-batch',
  'shell::fs::write',
  {
    files: [
      { path: '/srv/app/a.txt', content: 'alpha\n' },
      {
        path: '/srv/app/b.txt',
        content: {
          channel_id: 'chan-77aa2bcd-bb11-4e22-8f33-9d44ee55ff66',
          access_key: 'ak_streamed01',
          direction: 'write',
        },
        mode: '0600',
      },
    ],
  },
  {
    bytes_written: 26,
    path: '',
    files: [
      { path: '/srv/app/a.txt', bytes_written: 6 },
      { path: '/srv/app/b.txt', bytes_written: 20 },
    ],
  },
)

export const shellFsMkdirDone = base(
  'sh-mkdir',
  'shell::fs::mkdir',
  { path: '/srv/app/nested/dir', parents: true, mode: '0755' },
  { created: true, path: '/srv/app/nested/dir', already_existed: false },
)

export const shellFsRmDone = base(
  'sh-rm',
  'shell::fs::rm',
  { path: '/srv/app/tmp', recursive: true },
  wrapHarness({ removed: true, path: '/srv/app/tmp', was_present: true }),
)

export const shellFsMvDone = base(
  'sh-mv',
  'shell::fs::mv',
  { src: '/srv/app/old.txt', dst: '/srv/app/new.txt', overwrite: true },
  {
    moved: true,
    src: '/srv/app/old.txt',
    dst: '/srv/app/new.txt',
    overwrote: true,
  },
)

export const shellFsChmodDone = base(
  'sh-chmod',
  'shell::fs::chmod',
  { path: '/srv/app/scripts', mode: '0755', recursive: true },
  { entries_changed: 3, path: '/srv/app/scripts', recursive: true },
)

export const shellFsGrepDone = base(
  'sh-grep',
  'shell::fs::grep',
  {
    path: '/srv/app',
    pattern: 'TODO',
    ignore_case: true,
    include_glob: ['*.ts'],
  },
  wrapHarness({
    matches: [
      {
        path: '/srv/app/src/app.ts',
        line: 14,
        content: '  // TODO: wire shell UI',
      },
      {
        path: '/srv/app/src/jobs.ts',
        line: 3,
        content: '// TODO document job lifecycle',
      },
    ],
    truncated: false,
  }),
)

export const shellFsSedDone = base(
  'sh-sed',
  'shell::fs::sed',
  {
    files: ['/srv/app/a.txt', '/srv/app/b.txt'],
    pattern: 'foo',
    replacement: 'bar',
    regex: false,
  },
  {
    results: [
      { path: '/srv/app/a.txt', replacements: 2, success: true },
      {
        path: '/srv/app/b.txt',
        replacements: 0,
        success: false,
        error: 'permission denied',
      },
    ],
    total_replacements: 2,
  },
)

/** Flat SDK ErrorBody with a typed S-code (the direct error path). */
export const shellExecJailError = base(
  'sh-exec-jail-err',
  'shell::exec',
  { command: 'cat /etc/passwd', cwd: '/etc' },
  wrapHarness({ code: 'S215', message: 'cwd escapes jail: /etc' }),
)

/** The real-world path: harness gate-denial envelope whose `reason`
    embeds the S-code as text (`parseShellErrorDisplay` text-scan). */
export const shellStatusGateError = base(
  'sh-status-gate-err',
  'shell::status',
  { job_id: 'job-x' },
  {
    error: {
      kind: 'function_error',
      message: 'trigger_failed: IIIInvocationError: invocation_failed',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'gate_unavailable',
        function_id: 'shell::status',
        reason: 'trigger_failed: IIIInvocationError: S211: no such job: job-x',
      },
      content: [
        {
          type: 'text',
          text: 'trigger_failed: IIIInvocationError: S211: no such job: job-x',
        },
      ],
    },
  },
)

export const shellFixtures = [
  shellExecDone,
  shellExecArgvDone,
  shellExecSandboxDone,
  shellExecRunning,
  shellExecPending,
  shellExecBgDone,
  shellExecBgPending,
  shellStatusFinishedDone,
  shellStatusRunningDone,
  shellKillDone,
  shellKillAlreadyFinished,
  shellKillPending,
  shellListDone,
  shellConfigStatusDone,
  shellConfigStatusRejected,
  shellFsLsDone,
  shellFsStatDone,
  shellFsReadDone,
  shellFsWriteDone,
  shellFsWriteBatchDone,
  shellFsMkdirDone,
  shellFsRmDone,
  shellFsMvDone,
  shellFsChmodDone,
  shellFsGrepDone,
  shellFsSedDone,
  shellExecJailError,
  shellStatusGateError,
] as const
