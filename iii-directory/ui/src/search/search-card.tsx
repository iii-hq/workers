/**
 * Injected call-card view for `directory::search_functions`: the compact
 * candidate results, on the shared card chrome.
 */

import {
  ActionLine,
  Badge,
  Card,
  Chip,
  EmptyState,
  Eyebrow,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  MetaRow,
  TerminalCommandLine,
} from '@iii-dev/console-ui'
import { ChevronRight, Dot, SquareFunction } from 'lucide-react'
import type { ReactNode } from 'react'
import { kv } from '../lib/widgets'
import {
  type DiscoverCandidateView,
  type DiscoverInstallableView,
  type DiscoverView,
  discoverCapabilities,
  functionCount,
  isErrorOutput,
  parseDiscoverResponse,
} from './search'

function SectionHead({ children, count }: { children: ReactNode; count: number }) {
  return (
    <div className="dir-ui-search-head">
      <Eyebrow>{children}</Eyebrow>
      <span className="dir-ui-search-count">{count}</span>
    </div>
  )
}

/** One compact function candidate: id first, slim description on demand. */
function CandidateBlock({ candidate }: { candidate: DiscoverCandidateView }) {
  return (
    <details className="dir-ui-search-fn">
      <summary>
        <ChevronRight aria-hidden className="dir-ui-search-caret" />
        <SquareFunction aria-hidden className="dir-ui-search-sym" />
        <span className="dir-ui-search-fn-id">{candidate.function_id}</span>
      </summary>
      {candidate.description.length > 0 ? <div className="dir-ui-search-desc">{candidate.description}</div> : null}
    </details>
  )
}

/** One installable registry worker: header names it as NOT installed, a
 * description line, its matched candidates, and the exact install call. */
function InstallableSection({ worker }: { worker: DiscoverInstallableView }) {
  return (
    <section>
      <SectionHead count={worker.functions.length}>
        registry · {worker.name}
        {worker.version ? ` @ ${worker.version}` : ''}
        <Chip className="dir-ui-search-tag">not installed</Chip>
      </SectionHead>
      {worker.description.length > 0 ? <div className="dir-ui-search-desc">{worker.description}</div> : null}
      {worker.functions.map((fn) => (
        <ActionLine key={fn.function_id} icon={<SquareFunction />} tone="ink">
          <span className="dir-ui-search-fn-id">{fn.function_id}</span>
          {fn.description.length > 0 ? <span className="dir-ui-search-fn-desc">{fn.description}</span> : null}
        </ActionLine>
      ))}
      <TerminalCommandLine command={`compose::add { "worker": "${worker.name}" }`} copy />
    </section>
  )
}

function GuidanceDetails({ guidance }: { guidance: string }) {
  return (
    <details className="dir-ui-search-guidance">
      <summary>
        <ChevronRight aria-hidden className="dir-ui-search-caret" />
        guidance sent to the model
      </summary>
      <p>{guidance}</p>
    </details>
  )
}

export function DiscoverCard({ capabilities, view }: { capabilities: string[]; view: DiscoverView }) {
  const empty = view.workers.length === 0 && view.installable.length === 0
  return (
    <Card>
      <MetaRow
        items={kv([
          ['workers', view.workers.length],
          ['functions', functionCount(view)],
          ['installable', view.installable.length > 0 && view.installable.length],
          ['latency', `${Math.round(view.latency_ms)}ms`],
        ])}
      >
        <Badge variant="accent">search</Badge>
      </MetaRow>
      {capabilities.length > 0 ? (
        <section aria-label="capabilities">
          <SectionHead count={capabilities.length}>capabilities</SectionHead>
          {capabilities.map((capability, index) => (
            <ActionLine key={`${index}:${capability}`} icon={<Dot />} tone="ink">
              {capability}
            </ActionLine>
          ))}
        </section>
      ) : null}
      {empty ? (
        <EmptyState title="No functions matched" description={view.guidance} />
      ) : (
        <>
          {view.workers.map((worker) => (
            <section key={worker.namespace}>
              <SectionHead count={worker.functions.length}>worker · {worker.namespace}</SectionHead>
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
    </Card>
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
      return <DiscoverCard capabilities={discoverCapabilities(message.input)} view={view} />
    },
  }
}
