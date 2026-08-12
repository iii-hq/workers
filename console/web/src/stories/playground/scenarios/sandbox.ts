import {
  sandboxCreateDone,
  sandboxExecDone,
  sandboxFsWriteDone,
  sandboxStopDone,
} from '@/stories/fixtures/sandbox-fixtures'
import {
  makeBackend,
  streamAssistant,
  streamFcall,
  streamThought,
} from './helpers'

/** The daemon's flat S-code envelope, as it arrives: a JSON string in
    `error.message` (the wire parser digs it out of the text). */
const createS102Error = {
  error: {
    kind: 'function_error',
    message: JSON.stringify({
      type: 'transient',
      code: 'S102',
      message: 'microVM failed to start: sandbox host is out of CPU slots',
      retryable: true,
      fix: null,
      fix_note: 'retry shortly, or lower the requested cpus',
      docs_url: 'https://docs.example/s102',
    }),
  },
}

/**
 * The sandbox lifecycle: create → fs::write → exec → stop, then a second
 * `sandbox::create` that fails with the daemon's S102 transient error.
 */
export const sandboxLifecycle = makeBackend(
  'sandbox-lifecycle',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamThought(
      'booting a microVM, writing the script in, running it, then tearing the sandbox down…',
      { signal },
    )
    yield* streamFcall({
      functionId: 'sandbox::create',
      input: sandboxCreateDone.input,
      output: sandboxCreateDone.output,
      pendingApproval: true,
      approvalWaitMs: 1400,
      waitMs: 900,
      signal,
    })
    yield* streamFcall({
      functionId: 'sandbox::fs::write',
      input: sandboxFsWriteDone.input,
      output: sandboxFsWriteDone.output,
      waitMs: 400,
      signal,
    })
    yield* streamFcall({
      functionId: 'sandbox::exec',
      input: sandboxExecDone.input,
      output: sandboxExecDone.output,
      waitMs: 700,
      signal,
    })
    yield* streamFcall({
      functionId: 'sandbox::stop',
      input: sandboxStopDone.input,
      output: sandboxStopDone.output,
      waitMs: 300,
      signal,
    })
    yield* streamFcall({
      functionId: 'sandbox::create',
      input: sandboxCreateDone.input,
      output: createS102Error,
      waitMs: 600,
      signal,
    })
    yield* streamAssistant(
      'ran the script in a fresh microVM and stopped it; the follow-up create hit a transient S102 (host out of CPU slots) — safe to retry.',
      { signal },
    )
  },
)
