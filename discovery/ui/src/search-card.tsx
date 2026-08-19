/**
 * Injected call-card view for `discovery::search_functions`, mirroring the console's
 * `engine::functions::info` layout (meta row → ƒ line → description →
 * collapsible request schema). The console's card chrome is private to the
 * console bundle, so the small presentational pieces are ported here under
 * `discovery-search-*` classes — the accepted injected-UI pattern.
 */

import {
  Badge,
  CodeHighlight,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type JsonValue,
} from '@iii-dev/console-ui'
import { type ReactNode, useState } from 'react'
import {
  type DiscoverContractView,
  type DiscoverInstallableView,
  type DiscoverView,
  discoverQuery,
  functionCount,
  isErrorOutput,
  parseDiscoverResponse,
  schemaIsAny,
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

/** Engine-info style collapsible schema section: header row with a caret,
 * highlighted JSON mounted only while open. Unconstraining schemas render
 * as a flat `· any` row instead. */
function SchemaSection({ schema }: { schema: JsonValue }) {
  const [open, setOpen] = useState(false)
  if (schemaIsAny(schema)) {
    return <div className="discovery-search-schema-any">request · any</div>
  }
  return (
    <div className="discovery-search-schema">
      <button
        className="discovery-search-schema-toggle"
        onClick={() => setOpen((previous) => !previous)}
        type="button"
      >
        <span aria-hidden="true" className={`caret${open ? ' open' : ''}`}>
          ▸
        </span>
        request
      </button>
      {open ? (
        <div className="discovery-search-schema-body">
          <CodeHighlight code={JSON.stringify(schema, null, 2)} language="json" wrap />
        </div>
      ) : null}
    </div>
  )
}

/** One function as a collapsed `ƒ` row — a discover result carries whole
 * workers (dozens of contracts), so ids stay scannable and each description
 * plus schema is one click away instead of stacking meters of card. */
function ContractBlock({ contract }: { contract: DiscoverContractView }) {
  return (
    <details className="discovery-search-fn">
      <summary>
        <span aria-hidden="true" className="caret">
          ▸
        </span>
        <span aria-hidden="true" className="sym">
          ƒ
        </span>
        <span className="fn-id">{contract.function_id}</span>
      </summary>
      {contract.description.length > 0 ? (
        <div className="discovery-search-desc">{contract.description}</div>
      ) : null}
      <SchemaSection schema={contract.request_schema} />
    </details>
  )
}

/** One installable registry worker: header names it as NOT installed, a
 * description line, its matched contracts, and the exact install call. */
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
      {worker.functions.map((contract) => (
        <ContractBlock contract={contract} key={contract.function_id} />
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
  view,
}: {
  query: string | null
  view: DiscoverView
}) {
  const empty = view.workers.length === 0 && view.installable.length === 0
  return (
    <div className="discovery-search-card">
      <MetaRow>
        <Badge className="discovery-search-pill" variant="accent">
          discovery
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
              {worker.functions.map((contract) => (
                <ContractBlock contract={contract} key={contract.function_id} />
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
    id: 'discovery/page.js#search',
    isMatch: (functionId) => functionId === 'discovery::search_functions',
    tryRender: (message: FunctionTriggerMessage) => {
      if (message.pendingApproval) return null
      if (isErrorOutput(message.output)) return null
      const view = parseDiscoverResponse(message.output)
      if (!view) return null
      return <DiscoverCard query={discoverQuery(message.input)} view={view} />
    },
  }
}
