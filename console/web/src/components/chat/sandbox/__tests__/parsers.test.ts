import { describe, expect, it } from 'vitest'
import {
  createRequestSchema,
  createResponseSchema,
  execRequestSchema,
  execResponseSchema,
  extractFirstJsonObject,
  fsChmodRequestSchema,
  fsChmodResponseSchema,
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  fsLsRequestSchema,
  fsLsResponseSchema,
  fsMkdirRequestSchema,
  fsMkdirResponseSchema,
  fsMvRequestSchema,
  fsMvResponseSchema,
  fsReadResponseSchema,
  fsRmRequestSchema,
  fsRmResponseSchema,
  fsSedRequestSchema,
  fsSedResponseSchema,
  fsStatRequestSchema,
  fsStatResponseSchema,
  fsWriteRequestSchema,
  fsWriteResponseSchema,
  isSandboxErrorWire,
  listResponseSchema,
  parseSandboxErrorDisplay,
  runRequestSchema,
  runResponseSchema,
  sandboxErrorWireSchema,
  stopRequestSchema,
  stopResponseSchema,
  streamChannelRefSchema,
  unwrapEnvelope,
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

describe('unwrapEnvelope', () => {
  it('peels the harness envelope', () => {
    const payload = { sandbox_id: SB, stopped: true }
    expect(unwrapEnvelope(wrap(payload))).toEqual(payload)
  })

  it('passes through raw payloads unchanged', () => {
    const payload = { sandbox_id: SB, stopped: true }
    expect(unwrapEnvelope(payload)).toBe(payload)
  })

  it('is idempotent on flat payloads', () => {
    const payload = { foo: 1 }
    expect(unwrapEnvelope(unwrapEnvelope(payload))).toBe(payload)
  })

  it('passes through primitives and arrays', () => {
    expect(unwrapEnvelope(null)).toBe(null)
    expect(unwrapEnvelope('hi')).toBe('hi')
    const arr = [1, 2, 3]
    expect(unwrapEnvelope(arr)).toBe(arr)
  })

  it('does not peel objects that just happen to have a `content` field', () => {
    // The discriminator requires both `Array.isArray(content)` and the
    // presence of `details`; bare `content` strings/objects must pass
    // through as raw payloads.
    const payload = { content: 'inline string', size: 5 }
    expect(unwrapEnvelope(payload)).toBe(payload)
  })
})

describe('SandboxErrorWire schema', () => {
  it('accepts a fully populated S200 timeout payload', () => {
    const ok = sandboxErrorWireSchema.safeParse({
      type: 'execution',
      code: 'S200',
      message: 'exec timed out after 1500 ms',
      docs_url: 'https://example/S200',
      retryable: false,
      fix: null,
      fix_note: 'raise timeout_ms',
    })
    expect(ok.success).toBe(true)
  })

  it('rejects payloads whose code does not match /^S\\d{3}$/', () => {
    const bad = sandboxErrorWireSchema.safeParse({
      type: 'execution',
      code: 'X404',
      message: 'no',
    })
    expect(bad.success).toBe(false)
  })

  it('isSandboxErrorWire narrows correctly', () => {
    expect(
      isSandboxErrorWire({ type: 'config', code: 'S100', message: 'no' }),
    ).toBe(true)
    expect(isSandboxErrorWire({ stdout: '' })).toBe(false)
  })

  it('detects SandboxErrorWire inside harness envelope details', () => {
    const wire = { type: 'execution', code: 'S200', message: 'timed out' }
    expect(isSandboxErrorWire(wrap(wire))).toBe(true)
  })

  it('detects handler_error tagged payloads', () => {
    expect(
      isSandboxErrorWire({
        error: 'handler_error',
        type: 'filesystem',
        code: 'S210',
        message: 'path is required',
      }),
    ).toBe(true)
  })
})

describe('parseSandboxErrorDisplay', () => {
  it('parses flat SandboxErrorWire', () => {
    const out = parseSandboxErrorDisplay({
      type: 'execution',
      code: 'S200',
      message: 'timed out',
    })
    expect(out).toEqual({
      variant: 'wire',
      error: {
        type: 'execution',
        code: 'S200',
        message: 'timed out',
      },
    })
  })

  it('parses harness envelope with SandboxErrorWire in details', () => {
    const wire = {
      type: 'filesystem',
      code: 'S210',
      message: 'path is required',
      docs_url: 'https://example/S210',
    }
    const out = parseSandboxErrorDisplay(wrap(wire))
    expect(out?.variant).toBe('wire')
    if (out?.variant === 'wire') {
      expect(out.error.code).toBe('S210')
    }
  })

  it('parses translate function_error envelope with denial details', () => {
    const out = parseSandboxErrorDisplay({
      error: {
        kind: 'function_error',
        message:
          'trigger_failed: IIIInvocationError: invocation_failed: handler error',
        details: {
          schema_version: 1,
          status: 'denied',
          denied_by: 'gate_unavailable',
          function_id: 'sandbox::fs::write',
          reason: 'trigger_failed: policy unreachable',
        },
        content: [{ type: 'text', text: 'trigger_failed: policy unreachable' }],
      },
    })
    expect(out?.variant).toBe('invocation')
    if (out?.variant === 'invocation') {
      expect(out.error.deniedBy).toBe('gate_unavailable')
      expect(out.error.functionId).toBe('sandbox::fs::write')
      expect(out.error.title).toBe('Gate unavailable')
      expect(out.error.message).toBe('trigger_failed: policy unreachable')
    }
  })

  it('parses handler_error details inside function_error envelope', () => {
    const out = parseSandboxErrorDisplay({
      error: {
        kind: 'function_error',
        message: 'path is required',
        details: {
          error: 'handler_error',
          type: 'filesystem',
          code: 'S210',
          message: 'path is required',
          docs_url: 'https://example/S210',
        },
        content: [
          {
            type: 'text',
            text: '{"code":"S210","message":"path is required"}',
          },
        ],
      },
    })
    expect(out?.variant).toBe('wire')
    if (out?.variant === 'wire') {
      expect(out.error.code).toBe('S210')
    }
  })

  it('extracts embedded SandboxErrorWire JSON from error message text', () => {
    const envelope = {
      code: 'S220',
      type: 'filesystem',
      message: 'permission denied',
    }
    const out = parseSandboxErrorDisplay({
      error: {
        kind: 'function_error',
        message: `trigger_failed: handler error: ${JSON.stringify(envelope)}`,
        details: {
          schema_version: 1,
          status: 'denied',
          denied_by: 'gate_unavailable',
          function_id: 'sandbox::fs::write',
          reason: `trigger_failed: ${JSON.stringify(envelope)}`,
        },
        content: [],
      },
    })
    expect(out?.variant).toBe('wire')
    if (out?.variant === 'wire') {
      expect(out.error.code).toBe('S220')
    }
  })
})

describe('extractFirstJsonObject', () => {
  it('pulls the first balanced object from noisy text', () => {
    const embedded = extractFirstJsonObject(
      'trigger_failed: handler error: {"code":"S210","type":"filesystem","message":"bad path"} tail',
    )
    expect(embedded).toMatchObject({ code: 'S210', message: 'bad path' })
  })
})

describe('lifecycle schemas', () => {
  it('execRequest accepts the canonical example', () => {
    const ok = execRequestSchema.safeParse({
      sandbox_id: SB,
      cmd: 'node',
      args: ['/home/app/index.js'],
      env: { NODE_ENV: 'production' },
      timeout_ms: 300_000,
    })
    expect(ok.success).toBe(true)
  })

  it('execRequest accepts argv-only shape', () => {
    const ok = execRequestSchema.safeParse({
      sandbox_id: SB,
      argv: ['node', '/home/app/index.js'],
    })
    expect(ok.success).toBe(true)
  })

  it('execResponse round-trips through envelope unwrap', () => {
    const resp = {
      stdout: 'hi\n',
      stderr: '',
      exit_code: 0,
      timed_out: false,
      duration_ms: 12,
      success: true,
    }
    const wrapped = wrap(resp)
    const parsed = execResponseSchema.safeParse(unwrapEnvelope(wrapped))
    expect(parsed.success).toBe(true)
    if (parsed.success) expect(parsed.data.exit_code).toBe(0)
  })

  it('execResponse accepts null exit_code (timed out)', () => {
    const ok = execResponseSchema.safeParse({
      stdout: '',
      stderr: '',
      exit_code: null,
      timed_out: true,
      duration_ms: 100,
      success: false,
    })
    expect(ok.success).toBe(true)
  })

  it('runRequest accepts python lang + keep_sandbox', () => {
    const ok = runRequestSchema.safeParse({
      image: 'python',
      code: 'print(1)',
      lang: 'python',
      keep_sandbox: true,
    })
    expect(ok.success).toBe(true)
  })

  it('runResponse accepts optional sandbox_id', () => {
    const ok = runResponseSchema.safeParse({
      stdout: '',
      stderr: '',
      exit_code: 0,
      timed_out: false,
      duration_ms: 1,
      success: true,
      sandbox_id: SB,
    })
    expect(ok.success).toBe(true)
  })

  it('createRequest + response parse the canonical example', () => {
    expect(
      createRequestSchema.safeParse({
        image: 'node',
        memory_mb: 512,
        env: { NODE_ENV: 'production' },
        idle_timeout_secs: 600,
      }).success,
    ).toBe(true)
    expect(
      createResponseSchema.safeParse({ sandbox_id: SB, image: 'node' }).success,
    ).toBe(true)
  })

  it('stopRequest tolerates missing wait', () => {
    expect(stopRequestSchema.safeParse({ sandbox_id: SB }).success).toBe(true)
    expect(
      stopResponseSchema.safeParse({ sandbox_id: SB, stopped: true }).success,
    ).toBe(true)
  })

  it('listResponse accepts an empty list', () => {
    expect(listResponseSchema.safeParse({ sandboxes: [] }).success).toBe(true)
  })

  it('listResponse parses a populated table', () => {
    const ok = listResponseSchema.safeParse({
      sandboxes: [
        {
          sandbox_id: SB,
          name: 'worker',
          image: 'node',
          age_secs: 60,
          exec_in_progress: false,
          stopped: false,
        },
      ],
    })
    expect(ok.success).toBe(true)
  })
})

describe('fs schemas', () => {
  it('fs::ls request + response', () => {
    expect(
      fsLsRequestSchema.safeParse({ sandbox_id: SB, path: '/' }).success,
    ).toBe(true)
    expect(
      fsLsResponseSchema.safeParse({
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
      }).success,
    ).toBe(true)
  })

  it('fs::stat request + response', () => {
    expect(
      fsStatRequestSchema.safeParse({ sandbox_id: SB, path: '/a' }).success,
    ).toBe(true)
    expect(
      fsStatResponseSchema.safeParse({
        name: 'a',
        is_dir: false,
        size: 1,
        mode: '0644',
        mtime: 0,
        is_symlink: false,
      }).success,
    ).toBe(true)
  })

  it('fs::mkdir defaults', () => {
    expect(
      fsMkdirRequestSchema.safeParse({ sandbox_id: SB, path: '/d' }).success,
    ).toBe(true)
    expect(fsMkdirResponseSchema.safeParse({ created: true }).success).toBe(
      true,
    )
  })

  it('fs::read accepts inline string content', () => {
    const ok = fsReadResponseSchema.safeParse({
      content: 'hello',
      size: 5,
      mode: '0644',
      mtime: 0,
    })
    expect(ok.success).toBe(true)
  })

  it('fs::read accepts StreamChannelRef content', () => {
    const ok = fsReadResponseSchema.safeParse({
      content: { channel_id: 'ch1', access_key: 'key', direction: 'read' },
      size: 100,
      mode: '0644',
      mtime: 0,
    })
    expect(ok.success).toBe(true)
  })

  it('fs::write accepts UTF-8 content', () => {
    const ok = fsWriteRequestSchema.safeParse({
      sandbox_id: SB,
      path: '/a',
      content: 'console.log(1)\n',
    })
    expect(ok.success).toBe(true)
    expect(
      fsWriteResponseSchema.safeParse({ bytes_written: 15, path: '/a' })
        .success,
    ).toBe(true)
  })

  it('fs::write accepts StreamChannelRef content', () => {
    const ok = fsWriteRequestSchema.safeParse({
      sandbox_id: SB,
      path: '/a',
      content: { channel_id: 'ch1', access_key: 'key', direction: 'write' },
    })
    expect(ok.success).toBe(true)
  })

  it('fs::rm tolerates default recursive', () => {
    expect(
      fsRmRequestSchema.safeParse({ sandbox_id: SB, path: '/a' }).success,
    ).toBe(true)
    expect(fsRmResponseSchema.safeParse({ removed: true }).success).toBe(true)
  })

  it('fs::mv', () => {
    expect(
      fsMvRequestSchema.safeParse({ sandbox_id: SB, src: '/a', dst: '/b' })
        .success,
    ).toBe(true)
    expect(fsMvResponseSchema.safeParse({ moved: true }).success).toBe(true)
  })

  it('fs::chmod with uid/gid', () => {
    const ok = fsChmodRequestSchema.safeParse({
      sandbox_id: SB,
      path: '/a',
      mode: '0755',
      uid: 1000,
      gid: 1000,
      recursive: true,
    })
    expect(ok.success).toBe(true)
    expect(fsChmodResponseSchema.safeParse({ updated: 3 }).success).toBe(true)
  })

  it('fs::grep request + response with truncation flag', () => {
    expect(
      fsGrepRequestSchema.safeParse({
        sandbox_id: SB,
        path: '/src',
        pattern: 'TODO|FIXME',
        recursive: true,
        include_glob: ['**/*.ts'],
      }).success,
    ).toBe(true)
    expect(
      fsGrepResponseSchema.safeParse({
        matches: [{ path: '/a.ts', line: 1, content: '// TODO' }],
        truncated: true,
      }).success,
    ).toBe(true)
  })

  it('fs::grep response normalises legacy `file` alias to `path`', () => {
    const parsed = fsGrepResponseSchema.safeParse({
      matches: [{ file: '/legacy.ts', line: 7, content: 'TODO' }],
      truncated: false,
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.matches[0].path).toBe('/legacy.ts')
    }
  })

  it('fs::sed request + response', () => {
    expect(
      fsSedRequestSchema.safeParse({
        sandbox_id: SB,
        path: '/src',
        pattern: 'foo',
        replacement: 'bar',
      }).success,
    ).toBe(true)
    expect(
      fsSedResponseSchema.safeParse({
        results: [{ path: '/a.ts', replacements: 2, success: true }],
        total_replacements: 2,
      }).success,
    ).toBe(true)
  })

  it('fs::sed response surfaces per-file errors', () => {
    const parsed = fsSedResponseSchema.safeParse({
      results: [
        {
          path: '/a.ts',
          replacements: 0,
          success: false,
          error: 'permission denied',
        },
      ],
      total_replacements: 0,
    })
    expect(parsed.success).toBe(true)
    if (parsed.success) {
      expect(parsed.data.results[0].error).toBe('permission denied')
    }
  })

  it('streamChannelRef rejects unknown directions', () => {
    expect(
      streamChannelRefSchema.safeParse({
        channel_id: 'x',
        access_key: 'y',
        direction: 'sideways',
      }).success,
    ).toBe(false)
  })
})
