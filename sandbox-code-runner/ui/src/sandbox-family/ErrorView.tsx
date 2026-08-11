/**
 * The three failure cards for sandbox::* calls, ported from
 * console/web/src/components/chat/sandbox/ErrorView.tsx with two
 * family upgrades: the S003 card carries a `slots busy` chip (the
 * daemon serialises execs one-at-a-time per sandbox), and S200's
 * partial exec streams render through the SGR-aware `AnsiOutput`.
 */

import { Badge } from '@iii-dev/console-ui'
import { AnsiOutput } from './ansi'
import {
  execResponseSchema,
  type SandboxDispatchDenial,
  type SandboxErrorDisplay,
  type SandboxErrorWire,
  type SandboxInvocationError,
  safeParseResponse,
} from './parsers'

/**
 * Visualises the flat `{ type, code, message, docs_url, retryable,
 * fix, fix_note }` shape produced by `SandboxError::to_payload`. The
 * card is a warn-toned slab so it stands out from successful tool
 * cards without being as loud as the alert-red used for transport
 * errors. `S200` (exec timeout) deliberately leaks through here
 * rather than into `ExecView`: it carries no `ExecResponse`, so the
 * terminal chrome would be misleading — but its `fix` often carries
 * the partial streams, which re-parse as an `ExecResponse` and render
 * below the message.
 */
function execStreamsFromFix(error: SandboxErrorWire) {
  if (error.code !== 'S200' || error.fix == null) return null
  const parsed = safeParseResponse(execResponseSchema, error.fix)
  if (!parsed) return null
  const { stdout, stderr } = parsed
  if (!stdout && !stderr) return null
  return { stdout, stderr }
}

/** Only an http(s) URL earns the docs link — a wire payload must never
    mint a `javascript:`/`data:` href into the feed. */
function safeDocsUrl(raw: string | undefined): string | null {
  if (!raw) return null
  try {
    const { protocol } = new URL(raw)
    return protocol === 'http:' || protocol === 'https:' ? raw : null
  } catch {
    return null
  }
}

function ErrorView({ error }: { error: SandboxErrorWire }) {
  const retryable = error.retryable === true
  const streams = execStreamsFromFix(error)
  const docsUrl = safeDocsUrl(error.docs_url)
  return (
    <div className="cr-fam-card">
      <div className="cr-fam-err">
        <div className="cr-fam-err-head">
          <Badge variant="warn">{error.code}</Badge>
          <span className="cr-fam-err-type">{error.type}</span>
          {error.code === 'S003' ? <span className="cr-fam-chip warn">slots busy</span> : null}
          {retryable ? <Badge variant="accent">retryable</Badge> : null}
        </div>

        <pre className="cr-fam-err-msg">
          <code>{error.message}</code>
        </pre>

        {error.fix_note ? <div className="cr-fam-err-fix">{error.fix_note}</div> : null}

        {docsUrl ? (
          <a href={docsUrl} target="_blank" rel="noreferrer noopener" className="cr-fam-err-docs">
            docs ↗
          </a>
        ) : null}

        {streams ? (
          <div className="cr-fam-err-streams">
            <AnsiOutput stdout={streams.stdout} stderr={streams.stderr} />
          </div>
        ) : null}
      </div>
    </div>
  )
}

function InvocationErrorView({ error }: { error: SandboxInvocationError }) {
  const badge = error.deniedBy ?? 'error'
  const showDetailText = error.detailText && error.detailText !== error.message && error.detailText !== error.reason

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-err">
        <div className="cr-fam-err-head">
          <Badge variant="warn">{badge}</Badge>
          <span className="cr-fam-err-type">{error.title}</span>
        </div>

        {error.functionId ? (
          <div className="cr-fam-err-fn">
            <span className="k">function</span> <code>{error.functionId}</code>
          </div>
        ) : null}

        <pre className="cr-fam-err-msg">
          <code>{error.message}</code>
        </pre>

        {showDetailText ? (
          <pre className="cr-fam-err-detail">
            <code>{error.detailText}</code>
          </pre>
        ) : null}
      </div>
    </div>
  )
}

/**
 * A fail-closed dispatch-policy denial. Unlike a generic invocation failure,
 * this is structural and actionable: the calling agent's `functions` allow-list
 * doesn't cover the function. The card names the blocked id and tells the
 * operator exactly where to grant it (a workflow node's `agent.functions` /
 * the def's `default_functions`, or a session's `options.functions.allow`).
 */
function DispatchDeniedView({ denial }: { denial: SandboxDispatchDenial }) {
  const fn = denial.functionId
  return (
    <div className="cr-fam-card">
      <div className="cr-fam-err">
        <div className="cr-fam-err-head">
          <Badge variant="warn">denied</Badge>
          <span className="cr-fam-err-type">dispatch policy</span>
        </div>

        {fn ? (
          <div className="cr-fam-err-fn">
            <span className="k">blocked</span> <code>{fn}</code>
          </div>
        ) : null}

        <div className="cr-fam-err-body">
          {fn ? (
            <>
              This agent's allow-list doesn't include <code>{fn}</code>. Grant it where the agent is defined:
            </>
          ) : (
            <>This function isn't in the agent's allow-list. Grant it where the agent is defined:</>
          )}
          <ul className="cr-fam-err-list">
            <li>
              <span className="strong">workflow node</span> — its <code>agent.functions</code> (or the def's{' '}
              <code>default_functions</code>) narrows it out. Widen that, or drop the narrowing — nodes inherit the
              run's full reach by default.
            </li>
            <li>
              <span className="strong">chat / session</span> — add it to <code>options.functions.allow</code>.
            </li>
          </ul>
        </div>

        <pre className="cr-fam-err-detail">
          <code>{denial.message}</code>
        </pre>
      </div>
    </div>
  )
}

export function SandboxErrorView({ display }: { display: SandboxErrorDisplay }) {
  if (display.variant === 'wire') {
    return <ErrorView error={display.error} />
  }
  if (display.variant === 'dispatch-denied') {
    return <DispatchDeniedView denial={display.error} />
  }
  return <InvocationErrorView error={display.error} />
}
