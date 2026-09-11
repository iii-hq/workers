import type { FunctionTriggerMessage } from '@/types/chat'

const now = Date.now()
const SB = 'sb_abc123def456'

/** Harness agent envelope (half of fixtures use this). */
export function wrapHarness(details: unknown) {
  return {
    content: [
      { type: 'text' as const, text: JSON.stringify(details, null, 2) },
    ],
    details,
    terminate: true,
  }
}

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

export const sandboxExecDone = base(
  'sb-exec',
  'sandbox::exec',
  {
    sandbox_id: SB,
    cmd: 'ls',
    args: ['-la', '/workspace'],
    workdir: '/workspace',
    timeout_ms: 30_000,
  },
  wrapHarness({
    stdout:
      'total 8\ndrwxr-xr-x 2 root root 4096 May 26 10:00 .\ndrwxr-xr-x 1 root root 4096 May 26 09:58 ..\n-rw-r--r-- 1 root root   12 May 26 10:00 README.md\n',
    stderr: '',
    exit_code: 0,
    timed_out: false,
    success: true,
    duration_ms: 142,
  }),
)

export const sandboxExecRunning = base(
  'sb-exec-running',
  'sandbox::exec',
  {
    sandbox_id: SB,
    cmd: 'npm',
    args: ['test'],
    workdir: '/workspace',
  },
  undefined,
  { running: true },
)

export const sandboxExecPending = base(
  'sb-exec-pending',
  'sandbox::exec',
  {
    sandbox_id: SB,
    cmd: 'rm',
    args: ['-rf', '/tmp/scratch'],
  },
  undefined,
  { pendingApproval: true },
)

export const sandboxRunDone = base(
  'sb-run',
  'sandbox::run',
  {
    image: 'motia/node:20',
    lang: 'node',
    code: 'console.log("hello from sandbox")\n',
    keep_sandbox: false,
    timeout_ms: 60_000,
  },
  {
    stdout: 'hello from sandbox\n',
    stderr: '',
    exit_code: 0,
    timed_out: false,
    success: true,
    duration_ms: 890,
    sandbox_id: SB,
  },
)

export const sandboxCreateDone = base(
  'sb-create',
  'sandbox::create',
  {
    image: 'motia/node:20',
    cpus: 2,
    memory_mb: 512,
    name: 'dev-shell',
    network: true,
    idle_timeout_secs: 300,
    env: { NODE_ENV: 'development' },
  },
  wrapHarness({
    sandbox_id: SB,
    image: 'motia/node:20',
  }),
)

export const sandboxStopDone = base(
  'sb-stop',
  'sandbox::stop',
  { sandbox_id: SB, wait: true },
  { sandbox_id: SB, stopped: true },
)

export const sandboxListDone = base(
  'sb-list',
  'sandbox::list',
  {},
  wrapHarness({
    sandboxes: [
      {
        sandbox_id: SB,
        name: 'dev-shell',
        image: 'motia/node:20',
        age_secs: 120,
        exec_in_progress: false,
        stopped: false,
      },
      {
        sandbox_id: 'sb_old999',
        name: null,
        image: 'motia/python:3.12',
        age_secs: 3600,
        exec_in_progress: true,
        stopped: true,
      },
    ],
  }),
)

export const sandboxFsLsDone = base(
  'sb-ls',
  'sandbox::fs::ls',
  { sandbox_id: SB, path: '/workspace' },
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

export const sandboxFsStatDone = base(
  'sb-stat',
  'sandbox::fs::stat',
  { sandbox_id: SB, path: '/workspace/package.json' },
  wrapHarness({
    name: 'package.json',
    is_dir: false,
    size: 412,
    mode: '0644',
    mtime: Math.floor(now / 1000) - 60,
    is_symlink: false,
  }),
)

export const sandboxFsReadDone = base(
  'sb-read',
  'sandbox::fs::read',
  { sandbox_id: SB, path: '/workspace/src/index.ts' },
  {
    content: 'export const main = () => console.log("ok")\n',
    size: 48,
    mode: '0644',
    mtime: Math.floor(now / 1000) - 30,
  },
)

export const sandboxFsWriteDone = base(
  'sb-write',
  'sandbox::fs::write',
  {
    sandbox_id: SB,
    path: '/workspace/out.txt',
    content: 'hello',
    parents: true,
  },
  wrapHarness({ bytes_written: 5, path: '/workspace/out.txt' }),
)

export const sandboxFsMkdirDone = base(
  'sb-mkdir',
  'sandbox::fs::mkdir',
  {
    sandbox_id: SB,
    path: '/workspace/nested/dir',
    parents: true,
    mode: '0755',
  },
  { created: true },
)

export const sandboxFsRmDone = base(
  'sb-rm',
  'sandbox::fs::rm',
  { sandbox_id: SB, path: '/workspace/tmp', recursive: true },
  wrapHarness({ removed: true }),
)

export const sandboxFsMvDone = base(
  'sb-mv',
  'sandbox::fs::mv',
  {
    sandbox_id: SB,
    src: '/workspace/old.txt',
    dst: '/workspace/new.txt',
    overwrite: true,
  },
  { moved: true },
)

export const sandboxFsChmodDone = base(
  'sb-chmod',
  'sandbox::fs::chmod',
  { sandbox_id: SB, path: '/workspace/script.sh', mode: '0755' },
  { updated: 1 },
)

export const sandboxFsGrepDone = base(
  'sb-grep',
  'sandbox::fs::grep',
  {
    sandbox_id: SB,
    path: '/workspace',
    pattern: 'TODO',
    recursive: true,
    ignore_case: true,
  },
  wrapHarness({
    matches: [
      {
        path: '/workspace/src/app.ts',
        line: 14,
        content: '  // TODO: wire sandbox UI',
      },
      {
        path: '/workspace/README.md',
        line: 3,
        content: '- TODO document env vars',
      },
    ],
    truncated: false,
  }),
)

export const sandboxFsSedDone = base(
  'sb-sed',
  'sandbox::fs::sed',
  {
    sandbox_id: SB,
    files: ['/workspace/a.txt', '/workspace/b.txt'],
    pattern: 'foo',
    replacement: 'bar',
  },
  {
    results: [
      { path: '/workspace/a.txt', replacements: 2, success: true },
      {
        path: '/workspace/b.txt',
        replacements: 0,
        success: false,
        error: 'permission denied',
      },
    ],
    total_replacements: 2,
  },
)

export const sandboxExecError = base(
  'sb-exec-err',
  'sandbox::exec',
  { sandbox_id: SB, cmd: 'sleep', args: ['60'], timeout_ms: 100 },
  wrapHarness({
    type: 'sandbox_error',
    code: 'S200',
    message: 'command timed out after 100ms',
    docs_url: 'https://docs.example/s200',
    retryable: true,
    fix_note: 'increase timeout_ms or simplify the command',
    fix: {
      stdout: 'partial output\n',
      stderr: '',
      exit_code: null,
      timed_out: true,
      duration_ms: 100,
    },
  }),
)

export const sandboxFsWriteGateError = base(
  'sb-fs-write-gate',
  'sandbox::fs::write',
  { sandbox_id: SB, path: '/workspace/out.txt', content: 'hello\n' },
  {
    error: {
      kind: 'function_error',
      message:
        'trigger_failed: IIIInvocationError: invocation_failed: handler error',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'gate_unavailable',
        function_id: 'sandbox::fs::write',
        reason: 'trigger_failed: approval gate unreachable',
      },
      content: [
        {
          type: 'text',
          text: 'trigger_failed: approval gate unreachable',
        },
      ],
    },
  },
)

export const sandboxFixtures = [
  sandboxExecDone,
  sandboxExecRunning,
  sandboxExecPending,
  sandboxRunDone,
  sandboxCreateDone,
  sandboxStopDone,
  sandboxListDone,
  sandboxFsLsDone,
  sandboxFsStatDone,
  sandboxFsReadDone,
  sandboxFsWriteDone,
  sandboxFsMkdirDone,
  sandboxFsRmDone,
  sandboxFsMvDone,
  sandboxFsChmodDone,
  sandboxFsGrepDone,
  sandboxFsSedDone,
  sandboxExecError,
  sandboxFsWriteGateError,
] as const
