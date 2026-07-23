import { makeBackend, streamAssistant, streamFcall } from './helpers'

const BODY = `## triggered

approval was granted, the trigger fired, the downstream function returned
the expected ack. nothing else to do here.`

/**
 * Function trigger gated on user approval. The renderer shows approve/deny
 * affordances; this scenario auto-resolves after a short wait so we can
 * see the lifecycle pending → running → done in motion.
 */
export const pendingApproval = makeBackend(
  'pending-approval',
  async function* (_prompt, _mode, _model, opts) {
    const signal = opts?.signal
    yield* streamFcall({
      functionId: 'trigger::run',
      input: {
        triggerId: 'deploy::staging',
        actor: 'you',
        confirm: 'this will redeploy staging — proceed?',
      },
      output: {
        ack: true,
        deploymentId: 'dep_01HV2...',
      },
      pendingApproval: true,
      approvalWaitMs: 1800,
      waitMs: 600,
      signal,
    })
    yield* streamAssistant(BODY, { signal })
  },
)
