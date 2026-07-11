import { describe, expect, it } from 'vitest'
import {
  cleanShellErrorMessage,
  extractEmbeddedSCode,
  fsChmodRequestSchema,
  fsChmodResponseSchema,
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  fsLsRequestSchema,
  fsLsResponseSchema,
  fsMkdirResponseSchema,
  fsMvResponseSchema,
  fsPathRequestSchema,
  fsReadResponseSchema,
  fsRmResponseSchema,
  fsSedResponseSchema,
  fsStatResponseSchema,
  fsWriteRequestSchema,
  fsWriteResponseSchema,
  isShellFunction,
  jobRecordSchema,
  parseShellErrorDisplay,
  SHELL_FUNCTION_IDS,
  shellConfigStatusResponseSchema,
  shellErrorWireSchema,
  shellExecBgRequestSchema,
  shellExecBgResponseSchema,
  shellExecRequestSchema,
  shellExecResponseSchema,
  shellKillResponseSchema,
  shellListRequestSchema,
  shellListResponseSchema,
  shellStatusRequestSchema,
  shellStatusResponseSchema,
  stripHandlerPrefix,
  targetSchema,
} from '../parsers'

const SB = '00000000-0000-0000-0000-000000000000'

/** Build a `{ content, details, terminate }` envelope identical to
    what `harness/src/turn-orchestrator/agent-trigger.ts` produces. */
function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

/** Canonical finished JobRecord — all 10 keys present. */
const FINISHED_JOB = {
  id: 'job-1',
  argv: ['sleep', '5'],
  started_at_ms: 1_700_000_000_000,
  finished_at_ms: 1_700_000_005_000,
  status: 'finished',
  exit_code: 0,
  stdout: 'done\n',
  stderr: '',
  stdout_truncated: false,
  stderr_truncated: false,
}

/** Running record — `finished_at_ms`/`exit_code` are explicit nulls. */
const RUNNING_JOB = {
  ...FINISHED_JOB,
  id: 'job-2',
  finished_at_ms: null,
  status: 'running',
  exit_code: null,
  stdout: '',
}

describe('isShellFunction', () => {
  it('accepts all 16 shell ids', () => {
    expect(SHELL_FUNCTION_IDS).toHaveLength(16)
    for (const id of SHELL_FUNCTION_IDS) {
      expect(isShellFunction(id)).toBe(true)
    }
  })

  it('rejects near-misses from other families and misspellings', () => {
    expect(isShellFunction('sandbox::exec')).toBe(false)
    // The real id is dash-cased `shell::config-status`.
    expect(isShellFunction('shell::config_status')).toBe(false)
  })
})

describe('target schema', () => {
  it('accepts host and sandbox variants', () => {
    expect(targetSchema.safeParse({ kind: 'host' }).success).toBe(true)
    expect(
      targetSchema.safeParse({ kind: 'sandbox', sandbox_id: SB }).success,
    ).toBe(true)
  })

  it('rejects unknown kinds and sandbox without an id', () => {
    expect(targetSchema.safeParse({ kind: 'moon' }).success).toBe(false)
    expect(targetSchema.safeParse({ kind: 'sandbox' }).success).toBe(false)
  })

  it('is optional on requests but NOT nullable (serde default on non-Option)', () => {
    // Omitted ⇒ host default — fine.
    expect(shellExecRequestSchema.safeParse({ command: 'ls' }).success).toBe(
      true,
    )
    expect(fsPathRequestSchema.safeParse({ path: '/' }).success).toBe(true)
    // Explicit null is a server-side deserialize error — pin the rejection.
    expect(
      shellExecRequestSchema.safeParse({ command: 'ls', target: null }).success,
    ).toBe(false)
    expect(
      fsPathRequestSchema.safeParse({ path: '/', target: null }).success,
    ).toBe(false)
  })
})

describe('shell::exec request', () => {
  it('accepts the canonical fully-populated example', () => {
    const ok = shellExecRequestSchema.safeParse({
      command: 'ls',
      args: ['-la', '/tmp'],
      timeout_ms: 30_000,
      cwd: '/tmp',
      env: { PATH: '/bin' },
      stdin: 'hi',
      target: { kind: 'sandbox', sandbox_id: SB },
    })
    expect(ok.success).toBe(true)
  })

  it('keeps `args: []` distinguishable from absent (shell-line vs verbatim argv)', () => {
    const absent = shellExecRequestSchema.safeParse({ command: 'ls -la' })
    expect(absent.success).toBe(true)
    if (absent.success) expect(absent.data.args).toBeUndefined()

    const empty = shellExecRequestSchema.safeParse({ command: 'ls', args: [] })
    expect(empty.success).toBe(true)
    if (empty.success) expect(empty.data.args).toEqual([])

    const nul = shellExecRequestSchema.safeParse({ command: 'ls', args: null })
    expect(nul.success).toBe(true)
    if (nul.success) expect(nul.data.args).toBeNull()
  })

  it('swallows junk timeout_ms via .catch (server silently ignores non-u64)', () => {
    for (const junk of ['soon', -5, 1.5]) {
      const parsed = shellExecRequestSchema.safeParse({
        command: 'ls',
        timeout_ms: junk,
      })
      expect(parsed.success).toBe(true)
      if (parsed.success) expect(parsed.data.timeout_ms).toBeUndefined()
    }
    const valid = shellExecRequestSchema.safeParse({
      command: 'ls',
      timeout_ms: 1500,
    })
    expect(valid.success).toBe(true)
    if (valid.success) expect(valid.data.timeout_ms).toBe(1500)
  })

  it('accepts explicit nulls for cwd/env/stdin (Option + serde default)', () => {
    const ok = shellExecRequestSchema.safeParse({
      command: 'ls',
      cwd: null,
      env: null,
      stdin: null,
    })
    expect(ok.success).toBe(true)
  })

  it('rejects an array command (string-only on the server)', () => {
    expect(
      shellExecRequestSchema.safeParse({ command: ['ls', '-la'] }).success,
    ).toBe(false)
  })

  it('exec_bg request is the same schema object (alias, not copy)', () => {
    expect(shellExecBgRequestSchema).toBe(shellExecRequestSchema)
  })
})

describe('shell::exec / exec_bg responses', () => {
  it('accepts the canonical 7-key ExecResponse', () => {
    const ok = shellExecResponseSchema.safeParse({
      exit_code: 0,
      stdout: 'hi\n',
      stderr: '',
      duration_ms: 12,
      timed_out: false,
      stdout_truncated: false,
      stderr_truncated: false,
    })
    expect(ok.success).toBe(true)
  })

  it('accepts null exit_code on timeout', () => {
    const ok = shellExecResponseSchema.safeParse({
      exit_code: null,
      stdout: '',
      stderr: '',
      duration_ms: 1500,
      timed_out: true,
      stdout_truncated: false,
      stderr_truncated: true,
    })
    expect(ok.success).toBe(true)
  })

  it('rejects a payload missing a truncation key (contract canary)', () => {
    expect(
      shellExecResponseSchema.safeParse({
        exit_code: 0,
        stdout: '',
        stderr: '',
        duration_ms: 1,
        timed_out: false,
        stderr_truncated: false,
      }).success,
    ).toBe(false)
  })

  it('ExecBgResponse round-trips through the harness envelope', () => {
    const resp = { job_id: 'job-abc', argv: ['sleep', '5'] }
    const parsed = shellExecBgResponseSchema.safeParse(wrap(resp).details)
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.job_id).toBe('job-abc')
  })
})

describe('shell::status / shell::kill', () => {
  it('status request + full and running job records', () => {
    expect(
      shellStatusRequestSchema.safeParse({ job_id: 'job-1' }).success,
    ).toBe(true)
    expect(
      shellStatusResponseSchema.safeParse({ job: FINISHED_JOB }).success,
    ).toBe(true)
    expect(
      shellStatusResponseSchema.safeParse({ job: RUNNING_JOB }).success,
    ).toBe(true)
  })

  it('rejects wrong-case status enums (serde lowercase)', () => {
    expect(
      jobRecordSchema.safeParse({ ...FINISHED_JOB, status: 'Finished' })
        .success,
    ).toBe(false)
  })

  it('kill: not-running combo carries a reason', () => {
    const ok = shellKillResponseSchema.safeParse({
      job_id: 'job-1',
      killed: false,
      status: 'finished',
      reason: 'not running',
    })
    expect(ok.success).toBe(true)
  })

  it('kill: host kill omits the reason key entirely', () => {
    const parsed = shellKillResponseSchema.safeParse({
      job_id: 'job-1',
      killed: true,
      status: 'killed',
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.reason).toBeUndefined()
  })

  it('kill: sandbox kill carries the no-cancel-hook reason', () => {
    const ok = shellKillResponseSchema.safeParse({
      job_id: 'job-1',
      killed: true,
      status: 'killed',
      reason:
        'sandbox::exec has no cancel hook; the in-VM process will run until its timeout_ms expires',
    })
    expect(ok.success).toBe(true)
  })

  it('kill: reason is skip_serializing_if — null is rejected', () => {
    expect(
      shellKillResponseSchema.safeParse({
        job_id: 'job-1',
        killed: true,
        status: 'killed',
        reason: null,
      }).success,
    ).toBe(false)
  })
})

describe('shell::list / shell::config-status', () => {
  it('list request tolerates the engine-injected _caller_worker_id', () => {
    expect(
      shellListRequestSchema.safeParse({ _caller_worker_id: 'w1' }).success,
    ).toBe(true)
  })

  it('list response accepts empty and populated job tables', () => {
    expect(
      shellListResponseSchema.safeParse({ jobs: [], count: 0 }).success,
    ).toBe(true)
    expect(
      shellListResponseSchema.safeParse({
        jobs: [
          {
            id: 'job-2',
            status: 'running',
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: null,
            exit_code: null,
            stdout_truncated: false,
            stderr_truncated: false,
          },
        ],
        count: 1,
      }).success,
    ).toBe(true)
  })

  it('config-status accepts both outcomes; last_error is always present', () => {
    expect(
      shellConfigStatusResponseSchema.safeParse({
        last_outcome: 'applied',
        last_error: null,
        rejected_reloads: 0,
      }).success,
    ).toBe(true)
    expect(
      shellConfigStatusResponseSchema.safeParse({
        last_outcome: 'rejected',
        last_error: 'invalid allowlist pattern',
        rejected_reloads: 2,
      }).success,
    ).toBe(true)
    expect(
      shellConfigStatusResponseSchema.safeParse({
        last_outcome: 'applied',
        rejected_reloads: 0,
      }).success,
    ).toBe(false)
  })
})

describe('fs schemas', () => {
  const ENTRY = {
    name: 'a',
    is_dir: false,
    size: 1,
    mode: '0644',
    mtime: 0,
    is_symlink: false,
  }

  it('fs::ls request + response', () => {
    expect(
      fsLsRequestSchema.safeParse({
        path: '/',
        target: { kind: 'sandbox', sandbox_id: SB },
      }).success,
    ).toBe(true)
    expect(fsLsResponseSchema.safeParse({ entries: [ENTRY] }).success).toBe(
      true,
    )
  })

  it('fs::stat response is serde(transparent) — bare entry, no wrapper', () => {
    expect(fsStatResponseSchema.safeParse(ENTRY).success).toBe(true)
    expect(fsStatResponseSchema.safeParse({ entry: ENTRY }).success).toBe(false)
  })

  it('fs::mkdir minimal response fills serde defaults', () => {
    const parsed = fsMkdirResponseSchema.safeParse({ created: true })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.path).toBe('')
      expect(parsed.data.already_existed).toBe(false)
    }
  })

  it('fs::rm minimal response fills serde defaults', () => {
    const parsed = fsRmResponseSchema.safeParse({ removed: true })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.path).toBe('')
      expect(parsed.data.was_present).toBe(false)
    }
  })

  it('fs::mv minimal response fills serde defaults', () => {
    const parsed = fsMvResponseSchema.safeParse({ moved: true })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.src).toBe('')
      expect(parsed.data.dst).toBe('')
      expect(parsed.data.overwrote).toBe(false)
    }
  })

  it('fs::chmod request requires mode; uid/gid accept null', () => {
    expect(
      fsChmodRequestSchema.safeParse({
        path: '/a',
        mode: '0755',
        uid: null,
        gid: null,
      }).success,
    ).toBe(true)
    expect(fsChmodRequestSchema.safeParse({ path: '/a' }).success).toBe(false)
  })

  it('fs::chmod response accepts the legacy `updated` alias', () => {
    const parsed = fsChmodResponseSchema.safeParse({ updated: 3 })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.entries_changed).toBe(3)
      expect(parsed.data.path).toBe('')
      expect(parsed.data.recursive).toBe(false)
    }
  })

  it('fs::chmod response: entries_changed wins over updated; neither rejects', () => {
    const parsed = fsChmodResponseSchema.safeParse({
      entries_changed: 5,
      updated: 3,
      path: '/a',
      recursive: true,
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.entries_changed).toBe(5)
    expect(fsChmodResponseSchema.safeParse({ path: '/a' }).success).toBe(false)
  })

  it('fs::grep request + response normalise the legacy `file` key', () => {
    expect(
      fsGrepRequestSchema.safeParse({ path: '/src', pattern: 'TODO' }).success,
    ).toBe(true)
    const parsed = fsGrepResponseSchema.safeParse({
      matches: [{ file: '/legacy.ts', line: 7, content: 'TODO' }],
      truncated: false,
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.matches[0].path).toBe('/legacy.ts')
  })

  it('fs::sed response defaults absent per-file error to null', () => {
    const parsed = fsSedResponseSchema.safeParse({
      results: [{ path: '/a.ts', replacements: 2, success: true }],
      total_replacements: 2,
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.results[0].error).toBeNull()
  })

  it('fs::write accepts single inline, channel-ref, and batch request forms', () => {
    expect(
      fsWriteRequestSchema.safeParse({ path: '/a.txt', content: 'hello\n' })
        .success,
    ).toBe(true)
    expect(
      fsWriteRequestSchema.safeParse({
        path: '/big.bin',
        content: { channel_id: 'ch1', access_key: 'k', direction: 'write' },
      }).success,
    ).toBe(true)
    expect(
      fsWriteRequestSchema.safeParse({
        files: [
          { path: '/a', content: 'x' },
          {
            path: '/b',
            content: { channel_id: 'ch2', access_key: 'k' },
            mode: '0600',
          },
        ],
      }).success,
    ).toBe(true)
  })

  it('fs::write response defaults `files` to [] on single-file writes', () => {
    const single = fsWriteResponseSchema.safeParse({
      bytes_written: 6,
      path: '/a.txt',
    })
    expect(single.success).toBe(true)
    if (single.success) expect(single.data.files).toEqual([])

    const batch = fsWriteResponseSchema.safeParse({
      bytes_written: 10,
      path: '',
      files: [{ path: '/a', bytes_written: 4 }],
    })
    expect(batch.success).toBe(true)
    if (batch.success) expect(batch.data.files).toHaveLength(1)
  })

  it('fs::read content is ALWAYS a channel ref — inline strings rejected', () => {
    expect(
      fsReadResponseSchema.safeParse({
        content: 'hello',
        size: 5,
        mode: '0644',
        mtime: 0,
      }).success,
    ).toBe(false)
    expect(
      fsReadResponseSchema.safeParse({
        content: { channel_id: 'ch1', access_key: 'k', direction: 'read' },
        size: 100,
        mode: '0644',
        mtime: 1_700_000_000,
      }).success,
    ).toBe(true)
  })
})

describe('shellErrorWireSchema', () => {
  it('accepts the SDK ErrorBody with an S-code', () => {
    expect(
      shellErrorWireSchema.safeParse({
        code: 'S211',
        message: 'no such job: job-7',
        stacktrace: 'backtrace…',
      }).success,
    ).toBe(true)
  })

  it('rejects non-S codes (no false positives on {code,message} objects)', () => {
    expect(
      shellErrorWireSchema.safeParse({
        code: 'invocation_failed',
        message: 'handler error: argv: missing closing quote',
      }).success,
    ).toBe(false)
  })
})

describe('parseShellErrorDisplay', () => {
  it('lifts a flat S211 ErrorBody into the wire variant with its label', () => {
    const out = parseShellErrorDisplay({
      code: 'S211',
      message: 'no such job: job-7',
      stacktrace: 'backtrace…',
    })
    expect(out).toEqual({
      variant: 'wire',
      error: { type: 'not found', code: 'S211', message: 'no such job: job-7' },
    })
  })

  it('finds the ErrorBody inside the harness envelope', () => {
    const out = parseShellErrorDisplay(
      wrap({ code: 'S210', message: 'cwd must be absolute' }),
    )
    expect(out?.variant).toBe('wire')
    if (out?.variant === 'wire') {
      expect(out.error.code).toBe('S210')
      expect(out.error.type).toBe('bad request')
    }
  })

  it('falls back to a generic label for undocumented S-codes', () => {
    const out = parseShellErrorDisplay({ code: 'S999', message: 'mystery' })
    expect(out?.variant).toBe('wire')
    if (out?.variant === 'wire') expect(out.error.type).toBe('shell error')
  })

  it('text-scans the real-world denial envelope reason (the screenshot path)', () => {
    const out = parseShellErrorDisplay({
      error: {
        kind: 'function_error',
        message: 'trigger_failed: IIIInvocationError: invocation_failed',
        details: {
          schema_version: 1,
          status: 'denied',
          denied_by: 'gate_unavailable',
          function_id: 'shell::status',
          reason:
            'trigger_failed: IIIInvocationError: S211: no such job: job-x',
        },
        content: [
          {
            type: 'text',
            text: 'trigger_failed: IIIInvocationError: S211: no such job: job-x',
          },
        ],
      },
    })
    expect(out).toEqual({
      variant: 'wire',
      error: { type: 'not found', code: 'S211', message: 'no such job: job-x' },
    })
  })

  it('synthesizes S215 from an invocation_failed handler-error message (exec_bg path)', () => {
    const out = parseShellErrorDisplay(
      wrap({
        code: 'invocation_failed',
        message: 'handler error: S215: cwd escapes jail: /etc/passwd',
        stacktrace: 'backtrace…',
      }),
    )
    expect(out).toEqual({
      variant: 'wire',
      error: {
        type: 'filesystem access blocked',
        code: 'S215',
        message: 'cwd escapes jail: /etc/passwd',
      },
    })
  })

  it('keeps filesystem access metadata out of the readable error', () => {
    expect(
      cleanShellErrorMessage(
        'remote error (S215): path escapes the fs jail roots [/work]: /other filesystem_access_request={"v":1,"requested_root":"/other","attempted_path":"/other","error_code":"S215"}',
      ),
    ).toBe('path escapes the fs jail roots [/work]: /other')
  })

  it('delegates S-code-free denials to the sandbox invocation variant', () => {
    const out = parseShellErrorDisplay({
      schema_version: 1,
      status: 'denied',
      denied_by: 'user',
      function_id: 'shell::exec',
      reason: 'denied from the console approval prompt',
    })
    expect(out?.variant).toBe('invocation')
    if (out?.variant === 'invocation') {
      expect(out.error.title).toBe('Denied by user')
      expect(out.error.deniedBy).toBe('user')
      expect(out.error.message).toBe('denied from the console approval prompt')
    }
  })

  it('returns null on success payloads', () => {
    // ExecResponse — flat.
    expect(
      parseShellErrorDisplay({
        exit_code: 0,
        stdout: 'ok\n',
        stderr: '',
        duration_ms: 12,
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
      }),
    ).toBeNull()
    // KillResponse — its prose `reason` must not trip the text scan.
    expect(
      parseShellErrorDisplay({
        job_id: 'job-1',
        killed: false,
        status: 'finished',
        reason: 'not running',
      }),
    ).toBeNull()
    // ExecBgResponse — wrapped in the harness envelope.
    expect(
      parseShellErrorDisplay(wrap({ job_id: 'job-abc', argv: ['sleep', '5'] })),
    ).toBeNull()
    // LsResponse.
    expect(
      parseShellErrorDisplay({
        entries: [
          {
            name: 'a',
            is_dir: false,
            size: 1,
            mode: '0644',
            mtime: 0,
            is_symlink: false,
          },
        ],
      }),
    ).toBeNull()
  })
})

describe('extractEmbeddedSCode', () => {
  it('pulls the first S-code out of free text', () => {
    expect(
      extractEmbeddedSCode(
        'trigger_failed: IIIInvocationError: S211: no such job: job-x',
      ),
    ).toBe('S211')
    expect(extractEmbeddedSCode('handler error: S215: cwd escapes jail')).toBe(
      'S215',
    )
  })

  it('returns null when no word-boundary S-code is present', () => {
    expect(extractEmbeddedSCode('not running')).toBeNull()
    expect(extractEmbeddedSCode('XS211: glued prefix')).toBeNull()
  })
})

describe('stripHandlerPrefix', () => {
  it('strips the SDK Display prefixes', () => {
    expect(stripHandlerPrefix('handler error: S215: cwd escapes jail')).toBe(
      'S215: cwd escapes jail',
    )
    expect(stripHandlerPrefix('serialization error: invalid type')).toBe(
      'invalid type',
    )
  })

  it('strips the harness trigger_failed class prefix', () => {
    expect(
      stripHandlerPrefix(
        'trigger_failed: IIIInvocationError: S211: no such job',
      ),
    ).toBe('S211: no such job')
  })

  it('passes prefix-free messages through unchanged', () => {
    expect(stripHandlerPrefix('plain message')).toBe('plain message')
  })
})
