import { describe, expect, it } from 'vitest'
import {
  isWorkerFunction,
  safeParseRequest,
  safeParseResponse,
  statusState,
  unwrapEnvelope,
  WORKER_FUNCTION_IDS,
  type WorkerStatusResponse,
  workerAddRequestSchema,
  workerAddResponseSchema,
  workerClearResponseSchema,
  workerListRequestSchema,
  workerListResponseSchema,
  workerRemoveResponseSchema,
  workerStartRequestSchema,
  workerStartResponseSchema,
  workerStatusRequestSchema,
  workerStatusResponseSchema,
  workerStopRequestSchema,
  workerStopResponseSchema,
  workerUpdateResponseSchema,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

describe('isWorkerFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of WORKER_FUNCTION_IDS) {
      expect(isWorkerFunction(id)).toBe(true)
    }
  })

  it('rejects engine::workers::* (different family)', () => {
    expect(isWorkerFunction('engine::workers::list')).toBe(false)
    expect(isWorkerFunction('engine::workers::info')).toBe(false)
  })

  it('rejects unrelated ids', () => {
    expect(isWorkerFunction('sandbox::exec')).toBe(false)
    expect(isWorkerFunction('worker::')).toBe(false)
  })
})

describe('worker::list', () => {
  it('parses an empty request', () => {
    expect(safeParseRequest(workerListRequestSchema, {})).toEqual({})
  })

  it('parses a running_only filter', () => {
    expect(
      safeParseRequest(workerListRequestSchema, { running_only: true }),
    ).toEqual({ running_only: true })
  })

  it('parses the wrapped multi-worker payload from the screenshot', () => {
    const payload = {
      workers: [
        { name: 'iii-worker-manager', pid: null, running: true },
        { name: 'iii-directory', pid: 19052, running: true, version: '0.5.2' },
        { name: 'iii-stream', pid: null, running: true, version: '0.11.6' },
      ],
    }
    const parsed = safeParseResponse(workerListResponseSchema, wrap(payload))
    expect(parsed?.workers).toHaveLength(3)
    expect(parsed?.workers[1].pid).toBe(19052)
    expect(parsed?.workers[0].version).toBeUndefined()
  })

  it('parses an empty list', () => {
    expect(
      safeParseResponse(workerListResponseSchema, { workers: [] }),
    ).toEqual({ workers: [] })
  })
})

describe('worker::status', () => {
  it('round-trips the request example', () => {
    expect(
      safeParseRequest(workerStatusRequestSchema, { name: 'pdfkit' }),
    ).toEqual({ name: 'pdfkit' })
  })

  it('rejects a request missing name', () => {
    expect(safeParseRequest(workerStatusRequestSchema, {})).toBeNull()
  })

  it('round-trips a wrapped running StatusOutcome', () => {
    const outcome = {
      name: 'pdfkit',
      installed: true,
      worker_type: 'oci',
      running: true,
      pid: 28943,
      version: '1.0.0',
      logs_dir: '/Users/anderson/.iii/logs/pdfkit',
      stderr_tail: [],
      stdout_tail: ['[pdfkit] listening on :4101'],
      hint: 'worker is healthy; trigger it with `pdfkit::render`.',
    }
    const parsed = safeParseResponse(workerStatusResponseSchema, wrap(outcome))
    expect(parsed).toMatchObject({
      name: 'pdfkit',
      installed: true,
      running: true,
      pid: 28943,
      version: '1.0.0',
    })
    expect(parsed?.stdout_tail).toEqual(['[pdfkit] listening on :4101'])
  })

  it('parses null pid / version / logs_dir (engine builtin)', () => {
    const parsed = safeParseResponse(workerStatusResponseSchema, {
      name: 'iii-stream',
      installed: true,
      worker_type: 'builtin',
      running: true,
      pid: null,
      version: null,
      logs_dir: null,
      stderr_tail: [],
      stdout_tail: [],
      hint: 'engine builtin; managed by the engine process.',
    })
    expect(parsed?.pid).toBeNull()
    expect(parsed?.version).toBeNull()
    expect(parsed?.logs_dir).toBeNull()
  })

  it('defaults omitted tails to empty arrays', () => {
    const parsed = safeParseResponse(workerStatusResponseSchema, {
      name: 'pdfkit',
      installed: true,
      worker_type: 'oci',
      running: true,
      hint: 'healthy',
    })
    expect(parsed?.stderr_tail).toEqual([])
    expect(parsed?.stdout_tail).toEqual([])
  })

  it('parses a not-installed StatusOutcome', () => {
    const parsed = safeParseResponse(workerStatusResponseSchema, {
      name: 'ghost',
      installed: false,
      worker_type: 'not-installed',
      running: false,
      pid: null,
      version: null,
      logs_dir: null,
      stderr_tail: [],
      stdout_tail: [],
      hint: 'not declared in config.yaml; run `worker::add` first.',
    })
    expect(parsed?.installed).toBe(false)
    expect(parsed?.worker_type).toBe('not-installed')
  })
})

describe('statusState', () => {
  function outcome(over: Partial<WorkerStatusResponse>): WorkerStatusResponse {
    return {
      name: 'pdfkit',
      installed: true,
      worker_type: 'oci',
      running: false,
      pid: null,
      version: null,
      logs_dir: null,
      stderr_tail: [],
      stdout_tail: [],
      hint: 'x',
      ...over,
    }
  }

  it('derives "not-installed" when installed is false', () => {
    expect(
      statusState(outcome({ installed: false, worker_type: 'not-installed' })),
    ).toBe('not-installed')
  })

  it('derives "running" when installed and running', () => {
    expect(statusState(outcome({ running: true }))).toBe('running')
  })

  it('derives "provisioning" when down with both tails empty', () => {
    // No-tail down is the daemon's own "installed but not running and no logs
    // yet — likely still provisioning" branch, so the label matches its hint.
    // The presentation layer keeps this on the neutral pill variant (a rotated
    // tail could still hide a crash) — derivation just tracks the hint wording.
    expect(statusState(outcome({ running: false }))).toBe('provisioning')
  })

  it('derives "stopped" when down with a non-empty stderr tail', () => {
    expect(
      statusState(outcome({ running: false, stderr_tail: ['npm ERR! boom'] })),
    ).toBe('stopped')
  })

  it('derives "stopped" when down with only stdout output', () => {
    expect(
      statusState(outcome({ running: false, stdout_tail: ['booting…'] })),
    ).toBe('stopped')
  })
})

describe('worker::start', () => {
  it('parses a request', () => {
    expect(
      safeParseRequest(workerStartRequestSchema, { name: 'pdfkit' }),
    ).toEqual({ name: 'pdfkit' })
  })

  it('rejects a request missing name', () => {
    expect(safeParseRequest(workerStartRequestSchema, {})).toBeNull()
  })

  it('parses a response with null pid + port (engine builtin)', () => {
    expect(
      safeParseResponse(workerStartResponseSchema, {
        name: 'iii-stream',
        pid: null,
        port: null,
      }),
    ).toMatchObject({ name: 'iii-stream', pid: null, port: null })
  })

  it('parses a response with real pid + port', () => {
    expect(
      safeParseResponse(workerStartResponseSchema, {
        name: 'pdfkit',
        pid: 28943,
        port: 4101,
      }),
    ).toEqual({ name: 'pdfkit', pid: 28943, port: 4101 })
  })
})

describe('worker::stop', () => {
  it('accepts the request shape', () => {
    expect(
      safeParseRequest(workerStopRequestSchema, { name: 'pdfkit', yes: true }),
    ).toEqual({ name: 'pdfkit', yes: true })
  })

  it('parses stopped=true and stopped=false outcomes', () => {
    expect(
      safeParseResponse(workerStopResponseSchema, {
        name: 'pdfkit',
        stopped: true,
      }),
    ).toEqual({ name: 'pdfkit', stopped: true })
    expect(
      safeParseResponse(workerStopResponseSchema, {
        name: 'pdfkit',
        stopped: false,
      }),
    ).toEqual({ name: 'pdfkit', stopped: false })
  })
})

describe('worker::add', () => {
  it('accepts every WorkerSource variant', () => {
    expect(
      safeParseRequest(workerAddRequestSchema, {
        source: { kind: 'registry', name: 'pdfkit', version: '1.0.0' },
      })?.source.kind,
    ).toBe('registry')
    expect(
      safeParseRequest(workerAddRequestSchema, {
        source: { kind: 'oci', reference: 'ghcr.io/iii-hq/node:latest' },
      })?.source.kind,
    ).toBe('oci')
    expect(
      safeParseRequest(workerAddRequestSchema, {
        source: { kind: 'local', path: '/tmp/worker' },
      })?.source.kind,
    ).toBe('local')
  })

  it('rejects an unknown source kind', () => {
    expect(
      safeParseRequest(workerAddRequestSchema, {
        source: { kind: 'magic', name: 'x' },
      }),
    ).toBeNull()
  })

  it.each(['installed', 'already_current', 'repaired', 'replaced'] as const)(
    'parses the %s status',
    (status) => {
      const parsed = safeParseResponse(workerAddResponseSchema, {
        name: 'pdfkit',
        status,
        awaited_ready: true,
        config_path: '/x/iii.config.yaml',
      })
      expect(parsed?.status).toBe(status)
    },
  )
})

describe('worker::remove / clear / update', () => {
  it('parses remove outcomes (full + empty)', () => {
    expect(
      safeParseResponse(workerRemoveResponseSchema, {
        removed: ['a', 'b'],
      }),
    ).toEqual({ removed: ['a', 'b'] })
    expect(
      safeParseResponse(workerRemoveResponseSchema, { removed: [] }),
    ).toEqual({ removed: [] })
  })

  it('parses clear outcomes', () => {
    expect(
      safeParseResponse(workerClearResponseSchema, {
        cleared: ['pdfkit'],
      }),
    ).toEqual({ cleared: ['pdfkit'] })
  })

  it('parses update outcomes including version pairs', () => {
    const parsed = safeParseResponse(workerUpdateResponseSchema, {
      updated: [{ name: 'pdfkit', from_version: '1.0.0', to_version: '1.1.0' }],
    })
    expect(parsed?.updated[0].to_version).toBe('1.1.0')
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope', () => {
    const inner = { workers: [] }
    expect(unwrapEnvelope(wrap(inner))).toEqual(inner)
  })
})
