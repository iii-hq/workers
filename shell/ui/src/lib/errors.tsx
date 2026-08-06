/* The error cards — how a normalised `ErrorDisplay` (lib/error-display)
   renders. Ported from the console's sandbox/ErrorView.tsx when the shell
   function-trigger family moved into this worker's injected UI. */

import { Badge } from '@iii-dev/console-ui'
import { z } from 'zod'
import type {
  DispatchDenial,
  ErrorDisplay,
  ErrorWire,
  InvocationError,
} from './error-display'
import { AnsiOutput } from './terminal'

/** S200 exec-timeout errors smuggle the buffered streams in `fix`. */
const execStreamsSchema = z.object({
  stdout: z.string(),
  stderr: z.string(),
})

function execStreamsFromFix(error: ErrorWire) {
  if (error.code !== 'S200' || error.fix == null) return null
  const parsed = execStreamsSchema.safeParse(error.fix)
  if (!parsed.success) return null
  const { stdout, stderr } = parsed.data
  if (!stdout && !stderr) return null
  return { stdout, stderr }
}

function WireErrorView({ error }: { error: ErrorWire }) {
  const retryable = error.retryable === true
  const streams = execStreamsFromFix(error)
  return (
    <div className="shui-card">
      <div className="shui-slab warn">
        <div className="shui-row gap8">
          <Badge variant="warn">{error.code}</Badge>
          <span className="shui-err-label">{error.type}</span>
          {retryable ? (
            <Badge variant="accent" className="shui-pill-flat">
              retryable
            </Badge>
          ) : null}
        </div>

        <pre className="shui-pre out">
          <code>{error.message}</code>
        </pre>

        {error.fix_note ? (
          <div className="shui-note">{error.fix_note}</div>
        ) : null}

        {error.docs_url ? (
          <a
            href={error.docs_url}
            target="_blank"
            rel="noreferrer noopener"
            className="shui-doclink"
          >
            docs ↗
          </a>
        ) : null}

        {streams ? (
          <div className="shui-streams">
            <AnsiOutput stdout={streams.stdout} stderr={streams.stderr} />
          </div>
        ) : null}
      </div>
    </div>
  )
}

function InvocationErrorView({ error }: { error: InvocationError }) {
  const badge = error.deniedBy ?? 'error'
  const showDetailText =
    error.detailText &&
    error.detailText !== error.message &&
    error.detailText !== error.reason

  return (
    <div className="shui-card">
      <div className="shui-slab warn">
        <div className="shui-row gap8">
          <Badge variant="warn">{badge}</Badge>
          <span className="shui-err-label">{error.title}</span>
        </div>

        {error.functionId ? (
          <div className="shui-note plain">
            <span className="shui-err-label">function</span>{' '}
            <code className="t-ink">{error.functionId}</code>
          </div>
        ) : null}

        <pre className="shui-pre out">
          <code>{error.message}</code>
        </pre>

        {showDetailText ? (
          <pre className="shui-pre detail">
            <code>{error.detailText}</code>
          </pre>
        ) : null}
      </div>
    </div>
  )
}

/**
 * A fail-closed dispatch-policy denial. The card names the blocked id and
 * tells the operator exactly where to grant it.
 */
function DispatchDeniedView({ denial }: { denial: DispatchDenial }) {
  const fn = denial.functionId
  return (
    <div className="shui-card">
      <div className="shui-slab warn">
        <div className="shui-row gap8">
          <Badge variant="warn">denied</Badge>
          <span className="shui-err-label">dispatch policy</span>
        </div>

        {fn ? (
          <div className="shui-note plain">
            <span className="shui-err-label">blocked</span>{' '}
            <code className="t-ink">{fn}</code>
          </div>
        ) : null}

        <div className="shui-line">
          {fn ? (
            <>
              This agent's allow-list doesn't include <code>{fn}</code>. Grant
              it where the agent is defined:
            </>
          ) : (
            <>
              This function isn't in the agent's allow-list. Grant it where the
              agent is defined:
            </>
          )}
          <ul className="shui-denial-list">
            <li>
              <span className="t-ink">workflow node</span> — its{' '}
              <code className="t-ink">agent.functions</code> (or the def's{' '}
              <code className="t-ink">default_functions</code>) narrows it out.
              Widen that, or drop the narrowing — nodes inherit the run's full
              reach by default.
            </li>
            <li>
              <span className="t-ink">chat / session</span> — add it to{' '}
              <code className="t-ink">options.functions.allow</code>.
            </li>
          </ul>
        </div>

        <pre className="shui-pre detail">
          <code>{denial.message}</code>
        </pre>
      </div>
    </div>
  )
}

export function ErrorDisplayView({ display }: { display: ErrorDisplay }) {
  if (display.variant === 'wire') {
    return <WireErrorView error={display.error} />
  }
  if (display.variant === 'dispatch-denied') {
    return <DispatchDeniedView denial={display.error} />
  }
  return <InvocationErrorView error={display.error} />
}
