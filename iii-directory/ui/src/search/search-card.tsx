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
import { BookOpen, ChevronRight, Dot, SquareFunction, Zap } from 'lucide-react'
import type { ReactNode } from 'react'
import { kv } from '../lib/widgets'
import {
  type DiscoverCandidateView,
  type DiscoverInstallableView,
  type DiscoverSkillView,
  type DiscoverTriggerView,
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

/** One installed skill document: id (pass to `directory::skills::get`),
 * title, and its slim description. It documents a how-to, not a callable. */
function SkillBlock({ skill }: { skill: DiscoverSkillView }) {
  return (
    <details className="dir-ui-search-fn">
      <summary>
        <ChevronRight aria-hidden className="dir-ui-search-caret" />
        <BookOpen aria-hidden className="dir-ui-search-sym" />
        <span className="dir-ui-search-fn-id">{skill.id}</span>
        {skill.title && skill.title !== skill.id ? (
          <span className="dir-ui-search-fn-desc">{skill.title}</span>
        ) : null}
      </summary>
      {skill.description.length > 0 ? <div className="dir-ui-search-desc">{skill.description}</div> : null}
      <TerminalCommandLine command={`directory::skills::get { "id": "${skill.id}" }`} copy />
    </details>
  )
}

/** The binding config as one compact line; empty for `{}`/null. */
function configSummary(config: unknown): string {
  if (config === null || config === undefined) return ''
  if (typeof config === 'object' && Object.keys(config as object).length === 0) return ''
  return JSON.stringify(config)
}

/** One registered trigger binding: its type, the function it runs (and the
 * owning worker), and the binding config. The function is inspectable with
 * `engine::functions::info`. */
function TriggerBlock({ trigger }: { trigger: DiscoverTriggerView }) {
  const config = configSummary(trigger.config)
  return (
    <details className="dir-ui-search-fn">
      <summary>
        <ChevronRight aria-hidden className="dir-ui-search-caret" />
        <Zap aria-hidden className="dir-ui-search-sym" />
        <span className="dir-ui-search-fn-id">{trigger.triggerType}</span>
        <span className="dir-ui-search-fn-desc">
          → {trigger.functionId}
          {trigger.workerName ? ` · ${trigger.workerName}` : ''}
        </span>
      </summary>
      {config.length > 0 ? <div className="dir-ui-search-desc">{config}</div> : null}
      <TerminalCommandLine command={`engine::functions::info { "function_id": "${trigger.functionId}" }`} copy />
    </details>
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
  const empty =
    view.workers.length === 0 &&
    view.installable.length === 0 &&
    view.skills.length === 0 &&
    view.triggers.length === 0
  return (
    <Card>
      <MetaRow
        items={kv([
          ['workers', view.workers.length],
          ['functions', functionCount(view)],
          ['installable', view.installable.length > 0 && view.installable.length],
          ['skills', view.skills.length > 0 && view.skills.length],
          ['triggers', view.triggers.length > 0 && view.triggers.length],
          ['latency', `${Math.round(view.latency_ms)}ms`],
        ])}
      >
        <Badge variant="accent">{view.searchMode ?? 'search'}</Badge>
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
          {view.skills.length > 0 ? (
            <section aria-label="skills">
              <SectionHead count={view.skills.length}>installed skills</SectionHead>
              {view.skills.map((skill) => (
                <SkillBlock key={skill.id} skill={skill} />
              ))}
            </section>
          ) : null}
          {view.triggers.length > 0 ? (
            <section aria-label="triggers">
              <SectionHead count={view.triggers.length}>registered triggers</SectionHead>
              {view.triggers.map((trigger) => (
                <TriggerBlock key={trigger.id} trigger={trigger} />
              ))}
            </section>
          ) : null}
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
