/**
 * Injected call-card view for `directory::search_functions`, mirroring the console's
 * compact candidate results. The console's card chrome is private to the
 * console bundle, so the small presentational pieces are ported here under
 * `discovery-search-*` classes — the accepted injected-UI pattern.
 */

import {
  Badge,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
} from '@iii-dev/console-ui'
import { type ReactNode } from 'react'
import {
  type DiscoverCandidateView,
  type DiscoverInstallableView,
  type DiscoverView,
  discoverCapabilities,
  discoverQuery,
  functionCount,
  isErrorOutput,
  parseDiscoverResponse,
} from './search'

function MetaRow({ children }: { children: ReactNode }) {
  return <div className="discovery-search-meta">{children}</div>
}

function KvChip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="discovery-search-chip">
      <span className="k">{label}</span>
      <span className="v">{children}</span>
    </span>
  )
}

function ActionLine({
  symbol,
  tone,
  children,
}: {
  symbol: string
  tone: 'accent' | 'ink'
  children: ReactNode
}) {
  return (
    <div className="discovery-search-action">
      <span aria-hidden="true" className={`sym tone-${tone}`}>
        {symbol}
      </span>
      <div className="body">{children}</div>
    </div>
  )
}

/** One compact function candidate: id first, slim description on demand. */
function CandidateBlock({ candidate }: { candidate: DiscoverCandidateView }) {
  return (
    <details className="discovery-search-fn">
      <summary>
        <span aria-hidden="true" className="caret">
          ▸
        </span>
        <span aria-hidden="true" className="sym">
          ƒ
        </span>
        <span className="fn-id">{candidate.function_id}</span>
      </summary>
      {candidate.description.length > 0 ? (
        <div className="discovery-search-desc">{candidate.description}</div>
      ) : null}
    </details>
  )
}

/** One installable registry worker: header names it as NOT installed, a
 * description line, its matched candidates, and the exact install call. */
function InstallableSection({ worker }: { worker: DiscoverInstallableView }) {
  return (
    <section>
      <div className="discovery-search-section-head">
        <span>
          registry · {worker.name}
          {worker.version ? ` @ ${worker.version}` : ''}
          <span className="discovery-search-install-tag">not installed</span>
        </span>
        <span className="count">{worker.functions.length}</span>
      </div>
      {worker.description.length > 0 ? (
        <div className="discovery-search-install-desc">{worker.description}</div>
      ) : null}
      {worker.functions.map((fn) => (
        <div className="discovery-search-install-fn" key={fn.function_id}>
          <span aria-hidden="true" className="sym">
            ƒ
          </span>
          <span className="fn-id">{fn.function_id}</span>
          {fn.description.length > 0 ? (
            <span className="fn-desc">{fn.description}</span>
          ) : null}
        </div>
      ))}
      <div className="discovery-search-install-cmd">
        <span aria-hidden="true" className="sym">
          $
        </span>
        <code>{`worker::add { "source": { "kind": "registry", "name": "${worker.name}" }, "wait": false }`}</code>
      </div>
    </section>
  )
}

function GuidanceDetails({ guidance }: { guidance: string }) {
  return (
    <details className="discovery-search-guidance">
      <summary>
        <span aria-hidden="true" className="caret">
          ▸
        </span>
        guidance sent to the model
      </summary>
      <p>{guidance}</p>
    </details>
  )
}

export function DiscoverCard({
  query,
  capabilities,
  view,
}: {
  query: string | null
  capabilities: string[]
  view: DiscoverView
}) {
  const empty = view.workers.length === 0 && view.installable.length === 0
  return (
    <div className="discovery-search-card">
      <MetaRow>
        <Badge className="discovery-search-pill" variant="accent">
          search
        </Badge>
        <KvChip label="workers">{view.workers.length}</KvChip>
        <KvChip label="functions">{functionCount(view)}</KvChip>
        {view.installable.length > 0 ? (
          <KvChip label="installable">{view.installable.length}</KvChip>
        ) : null}
        <KvChip label="latency">{`${Math.round(view.latency_ms)}ms`}</KvChip>
      </MetaRow>
      {query ? (
        <ActionLine symbol="»" tone="ink">
          {query}
        </ActionLine>
      ) : null}
      {capabilities.length > 0 ? (
        <section aria-label="capabilities">
          <div className="discovery-search-section-head">
            <span>capabilities</span>
            <span className="count">{capabilities.length}</span>
          </div>
          {capabilities.map((capability, index) => (
            <ActionLine key={`${index}:${capability}`} symbol="·" tone="ink">
              {capability}
            </ActionLine>
          ))}
        </section>
      ) : null}
      {empty ? (
        <div className="discovery-search-empty">
          <div>· no functions matched</div>
          <p>{view.guidance}</p>
        </div>
      ) : (
        <>
          {view.workers.map((worker) => (
            <section key={worker.namespace}>
              <div className="discovery-search-section-head">
                <span>worker · {worker.namespace}</span>
                <span className="count">{worker.functions.length}</span>
              </div>
              {worker.functions.map((candidate) => (
                <CandidateBlock candidate={candidate} key={candidate.function_id} />
              ))}
            </section>
          ))}
          {view.installable.map((worker) => (
            <InstallableSection key={worker.name} worker={worker} />
          ))}
          <GuidanceDetails guidance={view.guidance} />
        </>
      )}
    </div>
  )
}

export function createSearchTriggerRenderer(): FunctionTriggerRenderer {
  return {
    id: 'iii-directory/page.js#search',
    isMatch: (functionId) => functionId === 'directory::search_functions',
    tryRender: (message: FunctionTriggerMessage) => {
      if (message.pendingApproval) return null
      if (isErrorOutput(message.output)) return null
      const view = parseDiscoverResponse(message.output)
      if (!view) return null
      return (
        <DiscoverCard
          capabilities={discoverCapabilities(message.input)}
          query={discoverQuery(message.input)}
          view={view}
        />
      )
    },
  }
}
