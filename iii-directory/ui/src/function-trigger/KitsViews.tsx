/**
 * Chat cards for the kit functions (screen G):
 *
 * - `directory::download-kit` / `plan-update` / `remove`: "Planning …"
 *   while running, then the plan summary (counts, warnings, permissions)
 *   and "Review install" — which opens the Directory page's Kits segment
 *   on that plan, or, on a console without `host.panels`, a compact review
 *   right in the card.
 * - `directory::kits::apply`: the three steps live while it runs, a summary
 *   when done, and — while a harness approval is pending — the plan
 *   summary in `tryRenderPreview`, so approve/deny is an informed choice.
 * - `directory::kits::check-updates`: kits with an update available.
 */

import { ActionLine, Badge, Button, Card, type Host, MetaRow } from '@iii-dev/console-ui'
import { ArrowUpCircle, Layers, ShieldAlert, TriangleAlert } from 'lucide-react'
import { useEffect, useState } from 'react'
import { errorText, kitsApi, useKitsChange } from '../kits/api'
import { functionsSummary, initialSteps, ownerLabel, workerLine } from '../kits/model'
import { StepList } from '../kits/parts'
import type { ApplyReport, ApplyStep, KitPlanResponse, Plan } from '../kits/types'
import { kv, Loading, Row, Rows, Section } from '../lib/widgets'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
  host?: Host
}

function obj(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

function isPlanResponse(value: unknown): value is KitPlanResponse {
  const o = obj(value)
  return typeof o.status === 'string' && typeof o.kit === 'string' && typeof o.message === 'string'
}

/** Open the Directory page's Kits segment on a plan or a kit. */
export function openKits(host: Host | undefined, context: { plan_id?: string; kit?: string }): boolean {
  if (!host?.panels?.open) return false
  host.panels.open({ pageId: 'directory', context: { collection: 'kits', ...context } })
  return true
}

function planTitle(plan: Plan): string {
  switch (plan.kind) {
    case 'install':
      return `Install ${plan.kit} ${plan.to ?? ''}`
    case 'update':
      return `Update ${plan.kit} ${plan.from ?? ''} → ${plan.to ?? ''}`
    case 'remove':
      return `Remove ${plan.kit} ${plan.from ?? ''}`
  }
}

function reviewLabel(plan: Plan): string {
  return plan.kind === 'install' ? 'Review install' : plan.kind === 'update' ? 'Review update' : 'Review removal'
}

/** What a plan will do, in card form. Shared by the plan cards and the
 * pending-approval preview of `directory::kits::apply`. */
export function PlanSummary({ plan, decisions }: { plan: Plan; decisions?: Record<string, unknown> }) {
  const collided = plan.files.filter((f) => f.collision)
  const conflicts = plan.files.filter((f) => f.merge === 'conflicts')
  const workers = plan.workers.filter((w) => w.action !== 'none')
  const otherWarnings = plan.warnings.filter((w) => !collided.some((f) => f.path === w.path))
  return (
    <>
      <MetaRow
        items={kv([
          ['profiles', plan.counts.agents],
          ['skills', plan.counts.skills],
          ['workers', plan.counts.workers],
          ['functions', plan.functions.length || null],
        ])}
      >
        {plan.major ? <Badge variant="alert">major</Badge> : null}
        {plan.blocking.length ? <Badge variant="alert">blocked</Badge> : null}
        {plan.warnings.length ? (
          <Badge variant="warn">
            {plan.warnings.length} warning{plan.warnings.length === 1 ? '' : 's'}
          </Badge>
        ) : null}
      </MetaRow>
      <ActionLine icon={<Layers />} tone="ink">
        {planTitle(plan)}
      </ActionLine>
      {plan.blocking.map((b) => (
        <ActionLine key={b.code} icon={<ShieldAlert />} tone="warn">
          {b.message}
        </ActionLine>
      ))}
      {collided.length ? (
        <Section label={`existing files replaced · ${collided.length}`}>
          <Rows>
            {collided.map((f) => {
              const d = obj(decisions)[f.path]
              const keep = d === 'keep' || obj(d).choice === 'keep'
              return (
                <Row
                  key={f.path}
                  mono
                  title={f.path}
                  description={f.collision ? ownerLabel(f.collision) : undefined}
                  meta={<Badge variant={keep ? 'default' : 'warn'}>{keep ? 'kept' : 'replaced'}</Badge>}
                />
              )
            })}
          </Rows>
        </Section>
      ) : null}
      {conflicts.length ? (
        <Section label={`conflicts · ${conflicts.length}`}>
          <Rows>
            {conflicts.map((f) => (
              <Row key={f.path} mono title={f.path} description="edited here and changed in the kit" />
            ))}
          </Rows>
        </Section>
      ) : null}
      {workers.length || plan.functions.length ? (
        <Section label="permissions">
          <Rows>
            {workers.map((w) => (
              <Row key={w.name} title={workerLine(w)} />
            ))}
            {plan.kind === 'install' && plan.functions.length ? <Row title={functionsSummary(plan.functions)} /> : null}
            {plan.kind === 'update' && plan.capabilities.functions_added?.length ? (
              <Row
                title={`+${plan.capabilities.functions_added.length} preloaded function${plan.capabilities.functions_added.length === 1 ? '' : 's'}`}
                description={plan.capabilities.functions_added.map((f) => `${f.agent}: ${f.function}`).join(' · ')}
              />
            ) : null}
            {(plan.capabilities.models_changed ?? [])
              .filter((m) => m.from)
              .map((m) => (
                <Row key={m.agent} title={`${m.agent}: ${m.from} → ${m.to}`} />
              ))}
          </Rows>
        </Section>
      ) : null}
      {otherWarnings.length ? (
        <Section label="also">
          <Rows>
            {otherWarnings.slice(0, 4).map((w) => (
              <Row key={w.code + (w.path ?? '')} title={w.message} />
            ))}
          </Rows>
        </Section>
      ) : null}
    </>
  )
}

/** A compact review in the card, for consoles that cannot open the page. */
function CompactReview({ host, plan }: { host?: Host; plan: Plan }) {
  const [state, setState] = useState<'idle' | 'applying' | 'done' | 'error'>('idle')
  const [message, setMessage] = useState('')
  const decisionsNeeded = plan.counts.decisions_required > 0
  if (!host || plan.blocking.length || decisionsNeeded) {
    return (
      <p className="dir-ui-empty">
        Review it in the ADE (Directory → Kits) or apply it with{' '}
        <code>iii trigger directory::kits::apply plan_id={plan.plan_id}</code>.
      </p>
    )
  }
  const apply = async () => {
    setState('applying')
    try {
      const out = await kitsApi(host).apply(plan.plan_id, {})
      setMessage(out.message)
      setState('done')
    } catch (e) {
      setMessage(errorText(e))
      setState('error')
    }
  }
  return (
    <div className="dir-ui-kit-card-actions">
      {state === 'done' || state === 'error' ? (
        <span className="dir-ui-empty">{message}</span>
      ) : (
        <Button variant="primary" size="sm" disabled={state === 'applying'} onClick={apply}>
          {state === 'applying'
            ? 'Applying…'
            : `${plan.kind === 'install' ? 'Install' : plan.kind === 'update' ? 'Update' : 'Remove'} with these defaults`}
        </Button>
      )}
    </div>
  )
}

export function KitPlanView({ input, output, running, host }: ViewProps) {
  const kit = str(obj(input).kit) ?? 'the kit'
  if (running) {
    return (
      <Card>
        <MetaRow items={kv([['kit', kit]])}>
          <Badge>planning…</Badge>
        </MetaRow>
        <ActionLine icon={<Layers />} tone="ink">
          Planning {kit}…
        </ActionLine>
        <Loading label="reading the kit and this project…" />
      </Card>
    )
  }
  if (!isPlanResponse(output)) return null
  if (output.status === 'applied' && output.report)
    return <ApplyReportCard report={output.report} message={output.message} />
  const plan = output.plan
  if (!plan) {
    return (
      <Card>
        <ActionLine icon={<Layers />} tone="ink">
          {output.message}
        </ActionLine>
      </Card>
    )
  }
  const canOpen = !!host?.panels?.open
  return (
    <Card>
      <PlanSummary plan={plan} />
      <div className="dir-ui-kit-card-actions">
        {canOpen ? (
          <Button variant="primary" size="sm" onClick={() => openKits(host, { plan_id: plan.plan_id })}>
            {reviewLabel(plan)}
          </Button>
        ) : (
          <CompactReview host={host} plan={plan} />
        )}
        <span className="dir-ui-fine">{plan.plan_id}</span>
      </div>
    </Card>
  )
}

function ApplyReportCard({ report, message }: { report: ApplyReport; message: string }) {
  return (
    <Card>
      <MetaRow
        items={kv([
          ['written', report.files_written.length + report.files_merged.length],
          ['removed', report.files_removed.length || null],
          ['kept', report.files_kept.length || null],
          ['workers', report.workers_added.length + report.workers_updated.length || null],
        ])}
      >
        <Badge variant="ok">applied</Badge>
      </MetaRow>
      <ActionLine icon={<Layers />} tone="ink">
        {message}
      </ActionLine>
      {report.agents.length ? (
        <Section label="profiles from this kit">
          <Rows>
            {report.agents.map((a) => (
              <Row key={a} mono title={a} />
            ))}
          </Rows>
        </Section>
      ) : null}
    </Card>
  )
}

/** `directory::kits::apply` while it runs: the steps, live. */
function ApplyRunning({ host, planId }: { host?: Host; planId: string }) {
  return (
    <Card>
      <MetaRow items={kv([['plan', planId]])}>
        <Badge>applying…</Badge>
      </MetaRow>
      {host ? <LiveSteps host={host} planId={planId} /> : <Loading label="applying the plan…" />}
    </Card>
  )
}

function LiveSteps({ host, planId }: { host: Host; planId: string }) {
  const [steps, setSteps] = useState<ApplyStep[] | null>(null)
  useKitsChange(host, `card-${planId}`, (change) => {
    if (change.op === 'progress' && change.plan_id === planId && change.steps) setSteps(change.steps)
  })
  return steps ? <StepList steps={steps} /> : <Loading label="re-checking the plan…" />
}

export function KitApplyView({ input, output, running, host }: ViewProps) {
  const planId = str(obj(input).plan_id) ?? ''
  if (running) return <ApplyRunning host={host} planId={planId} />
  if (!isPlanResponse(output)) return null
  if (output.status === 'applied' && output.report)
    return <ApplyReportCard report={output.report} message={output.message} />
  return <KitPlanView input={{ kit: output.kit }} output={output} host={host} />
}

/** Pending harness approval of `directory::kits::apply`: show what the
 * plan does so the human approves or denies it informed. */
export function KitApplyPreview({ input, host }: { input: unknown; host?: Host }) {
  const planId = str(obj(input).plan_id)
  const decisions = obj(input).decisions
  const [plan, setPlan] = useState<Plan | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!host || !planId) return
    let cancelled = false
    host.iii
      .trigger<{ plan: Plan }>('directory::kits::plan', { plan_id: planId })
      .then((r) => {
        if (!cancelled) setPlan(r.plan)
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e))
      })
    return () => {
      cancelled = true
    }
  }, [host, planId])
  if (!planId) return null
  return (
    <Card>
      <MetaRow items={kv([['plan', planId]])}>
        <Badge variant="warn">awaiting approval</Badge>
      </MetaRow>
      {plan ? (
        <PlanSummary plan={plan} decisions={obj(decisions)} />
      ) : error ? (
        <ActionLine icon={<TriangleAlert />} tone="warn">
          {error}
        </ActionLine>
      ) : (
        <Loading label="loading the plan…" />
      )}
      {plan ? (
        <div className="dir-ui-kit-card-actions">
          {host?.panels?.open ? (
            <Button variant="ghost" size="sm" onClick={() => openKits(host, { plan_id: planId })}>
              Open the full review
            </Button>
          ) : null}
          <span className="dir-ui-fine">
            steps:{' '}
            {initialSteps(plan)
              .map((s) => s.label)
              .join(' → ')}
          </span>
        </div>
      ) : null}
    </Card>
  )
}

export function KitUpdatesView({ output, running, host }: ViewProps) {
  if (running) {
    return (
      <Card>
        <ActionLine icon={<ArrowUpCircle />} tone="ink">
          Checking installed kits for updates…
        </ActionLine>
        <Loading label="asking the registry…" />
      </Card>
    )
  }
  const o = obj(output)
  const kits = Array.isArray(o.kits) ? (o.kits as Record<string, unknown>[]) : null
  if (!kits) return null
  const available = kits.filter((k) => str(k.available))
  return (
    <Card>
      <MetaRow
        items={kv([
          ['installed', kits.length],
          ['updates', available.length],
        ])}
      >
        {str(o.error) ? <Badge variant="warn">check failed</Badge> : null}
      </MetaRow>
      {available.length === 0 ? (
        <ActionLine icon={<ArrowUpCircle />} tone="ink">
          Every installed kit is up to date.
        </ActionLine>
      ) : (
        <Section label={`updates available · ${available.length}`}>
          <Rows>
            {available.map((k) => (
              <Row
                key={String(k.kit)}
                mono
                title={`${k.kit} ${k.current} → ${k.available}`}
                meta={
                  <>
                    {k.major ? <Badge variant="alert">major</Badge> : null}
                    {host?.panels?.open ? (
                      <Button variant="ghost" size="sm" onClick={() => openKits(host, { kit: String(k.kit) })}>
                        View
                      </Button>
                    ) : null}
                  </>
                }
              />
            ))}
          </Rows>
        </Section>
      )}
    </Card>
  )
}

/** `directory::kits::list`: installed kits and pending reviews. */
export function KitListView({ output, running }: ViewProps) {
  if (running) return <Loading label="listing kits…" />
  const o = obj(output)
  const kits = Array.isArray(o.kits) ? (o.kits as Record<string, unknown>[]) : null
  if (!kits) return null
  const pending = Array.isArray(o.pending) ? o.pending.length : 0
  return (
    <Card>
      <MetaRow
        items={kv([
          ['installed', kits.length],
          ['to review', pending || null],
        ])}
      />
      <Rows>
        {kits.map((k) => {
          const update = obj(k.update)
          return (
            <Row
              key={String(k.kit)}
              mono
              title={String(k.kit)}
              meta={
                <>
                  <span className="dir-ui-fine">{String(k.version)}</span>
                  {str(update.available) ? <Badge variant="accent">↑ {String(update.available)}</Badge> : null}
                </>
              }
            />
          )
        })}
      </Rows>
    </Card>
  )
}
