/**
 * The Kits segment of the Directory page. The sidebar lists plans waiting
 * for review and installed kits; the workspace shows the overview (screen
 * A), one installed kit (B), or a plan under review (C install, D update,
 * F removal). Everything refreshes on `directory::kits::on-change`.
 */

import {
  Button,
  EmptyState,
  Eyebrow,
  type Host,
  Input,
  List,
  ListItem,
  PageSidebar,
  Skeleton,
  StatusPanel,
  TerminalCommandLine,
  useConfirm,
} from '@iii-dev/console-ui'
import { useContainerNarrow } from '@iii-dev/console-ui/hooks'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { ArrowUpCircle, Clock, Layers, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { ago } from '../lib/format'
import { errorText, kitsApi, type PlanRecord, useKitsChange } from './api'
import { InstallReview } from './InstallReview'
import { KitDetail } from './KitDetail'
import { KitMark } from './parts'
import { RemoveReview } from './RemoveReview'
import type { InstalledKit, KitsListing, Plan, PlanSummary } from './types'
import { UpdateReview } from './UpdateReview'

const NARROW_BELOW = 850

export type KitsRoute =
  | { view: 'home' }
  | { view: 'kit'; kit: string }
  | { view: 'plan'; planId: string }
  | { view: 'install' }

export function KitsView({
  host,
  nav,
  panelSide,
  route,
  onRoute,
  onOpenEntry,
  active,
}: {
  host: Host
  nav: ReactNode
  panelSide: 'left' | 'right'
  route: KitsRoute
  onRoute: (route: KitsRoute) => void
  onOpenEntry: (collection: 'agents' | 'skills', key: string) => void
  active: boolean
}) {
  const [listing, setListing] = useState<KitsListing | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [checking, setChecking] = useState(false)
  // A replanned apply lands on a new plan id; its reason rides along.
  const [notices, setNotices] = useState<Record<string, string>>({})
  const { ref, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const { confirm, dialog } = useConfirm()
  const api = kitsApi(host)

  const refresh = useCallback(() => {
    kitsApi(host)
      .list()
      .then((l) => {
        setListing(l)
        setListError(null)
      })
      .catch((e) => setListError(errorText(e)))
  }, [host])
  useEffect(() => {
    if (active) refresh()
  }, [active, refresh])
  useKitsChange(host, 'kits-view', (change) => {
    if (change.op !== 'progress') refresh()
  })

  /** Ask for a plan (install / update / removal), then open it. */
  const planThen = async (key: string, make: () => Promise<{ plan?: Plan; message: string; status: string }>) => {
    setBusy(key)
    setActionError(null)
    try {
      const out = await make()
      if (out.plan) onRoute({ view: 'plan', planId: out.plan.plan_id })
      else setActionError(out.message)
      refresh()
    } catch (e) {
      setActionError(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  const checkUpdates = async () => {
    setChecking(true)
    setActionError(null)
    try {
      await api.checkUpdates()
    } catch (e) {
      setActionError(errorText(e))
    } finally {
      setChecking(false)
      refresh()
    }
  }

  const reviewUpdate = (kit: string) => {
    const pending = listing?.pending.find((p) => p.kit === kit && p.kind === 'update')
    if (pending) onRoute({ view: 'plan', planId: pending.plan_id })
    else planThen(kit, () => api.planUpdate(kit))
  }

  const showSide = !narrow || route.view === 'home'
  const showMain = !narrow || route.view !== 'home'

  return (
    <div
      className={`dir-ui-browser dir-ui-kits${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
      ref={ref}
    >
      {dialog}
      {showSide ? (
        <PageSidebar
          label="kits"
          side={panelSide}
          collapsible
          storageKey="iii-directory:navigation"
          defaultWidth={300}
          narrow={narrow}
          className="dir-ui-side"
          header={nav}
          collapsedActions={undefined}
        >
          <div className="dir-ui-side-top">
            <Button
              variant="ghost"
              size="sm"
              className="dir-ui-new-btn"
              onClick={() => onRoute({ view: 'install' })}
              aria-label="install a kit"
            >
              <span className="dir-ui-new-mark">
                <Plus />
              </span>
              <span>Install a kit</span>
            </Button>
            <div className="dir-ui-kit-side-actions">
              {listing ? (
                <span className="dir-ui-count" aria-live="polite">
                  {listing.kits.length} kit{listing.kits.length === 1 ? '' : 's'}
                  {listing.pending.length ? ` · ${listing.pending.length} to review` : ''}
                </span>
              ) : (
                <span />
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={checkUpdates}
                disabled={checking}
                title="Check installed kits for updates"
              >
                <RefreshCw aria-hidden className={checking ? uiClasses.spin : undefined} />
                {checking ? 'Checking…' : 'Updates'}
              </Button>
            </div>
          </div>
          <div className="dir-ui-side-scroll">
            {listError ? (
              <div className="dir-ui-error">
                <StatusPanel variant="alert" headline="Kits could not be loaded." detail={listError} />
                <Button variant="ghost" size="sm" onClick={refresh}>
                  Retry
                </Button>
              </div>
            ) : listing === null ? (
              <div className="dir-ui-skel" aria-hidden>
                {[0, 1, 2].map((i) => (
                  <div key={i} className="dir-ui-skel-row">
                    <Skeleton style={{ width: '60%' }} />
                    <Skeleton style={{ width: '85%' }} />
                  </div>
                ))}
              </div>
            ) : (
              <>
                {listing.pending.length > 0 ? (
                  <div className="dir-ui-kit-side-group">
                    <Eyebrow as="div" className="dir-ui-kit-side-head">
                      Needs review
                    </Eyebrow>
                    <List>
                      {listing.pending.map((p) => (
                        <PendingRow
                          key={p.plan_id}
                          plan={p}
                          active={route.view === 'plan' && route.planId === p.plan_id}
                          onOpen={() => onRoute({ view: 'plan', planId: p.plan_id })}
                        />
                      ))}
                    </List>
                  </div>
                ) : null}
                <div className="dir-ui-kit-side-group">
                  <Eyebrow as="div" className="dir-ui-kit-side-head">
                    Installed
                  </Eyebrow>
                  {listing.kits.length === 0 ? (
                    <p className="dir-ui-kit-fine dir-ui-kit-pad">No kits yet.</p>
                  ) : (
                    <List>
                      {listing.kits.map((k) => (
                        <ListItem
                          key={k.kit}
                          className={`dir-ui-nav-row${route.view === 'kit' && route.kit === k.kit ? ' active' : ''}`}
                          aria-current={route.view === 'kit' && route.kit === k.kit ? 'true' : undefined}
                          onClick={() => onRoute({ view: 'kit', kit: k.kit })}
                          leading={
                            <span className="dir-ui-nav-ico" aria-hidden>
                              <Layers />
                            </span>
                          }
                          label={<span className="dir-ui-mono">{k.kit}</span>}
                          description={
                            <span className="dir-ui-nav-fine">
                              {k.version}
                              {k.update?.available ? ` · ↑ ${k.update.available}` : ''}
                              {k.files.edited ? ` · ${k.files.edited} edited` : ''}
                            </span>
                          }
                        />
                      ))}
                    </List>
                  )}
                </div>
              </>
            )}
          </div>
        </PageSidebar>
      ) : null}
      {showMain ? (
        <section className="dir-ui-doc dir-ui-kit-doc" aria-label="kits workspace">
          {actionError ? (
            <div className="dir-ui-banner" role="alert">
              <StatusPanel
                variant="alert"
                headline="That did not work."
                detail={actionError}
                action={
                  <Button variant="ghost" size="sm" onClick={() => setActionError(null)}>
                    Dismiss
                  </Button>
                }
              />
            </div>
          ) : null}
          {route.view === 'plan' ? (
            <PlanScreen
              key={route.planId}
              host={host}
              planId={route.planId}
              onBack={() => onRoute({ view: 'home' })}
              onRoute={onRoute}
              onOpenEntry={onOpenEntry}
              notice={notices[route.planId] ?? null}
              onNotice={(planId, message) => setNotices((n) => ({ ...n, [planId]: message }))}
              confirmDiscard={() =>
                confirm({
                  title: 'Discard this plan?',
                  description: 'Nothing was written; you can plan it again any time.',
                  confirmLabel: 'Discard',
                })
              }
            />
          ) : route.view === 'kit' ? (
            <KitDetail
              key={route.kit}
              host={host}
              kit={route.kit}
              busy={busy}
              onBack={() => onRoute({ view: 'home' })}
              onReviewUpdate={reviewUpdate}
              onRemove={(kit) => planThen(kit, () => api.remove(kit))}
              onOpenEntry={onOpenEntry}
            />
          ) : (
            <Overview
              host={host}
              listing={listing}
              busy={busy}
              checking={checking}
              installing={route.view === 'install'}
              onInstall={(ref) => planThen(`install:${ref}`, () => api.download(ref))}
              onCheck={checkUpdates}
              onOpenKit={(kit) => onRoute({ view: 'kit', kit })}
              onOpenPlan={(planId) => onRoute({ view: 'plan', planId })}
              onReviewUpdate={reviewUpdate}
              onCancelInstall={() => onRoute({ view: 'home' })}
              narrow={narrow}
              onBack={() => onRoute({ view: 'home' })}
            />
          )}
        </section>
      ) : null}
    </div>
  )
}

function planVerb(p: PlanSummary): string {
  if (p.kind === 'install') return `Install ${p.to ?? ''}`
  if (p.kind === 'update') return `Update ${p.from ?? ''} → ${p.to ?? ''}`
  return `Remove ${p.from ?? ''}`
}

function PendingRow({ plan, active, onOpen }: { plan: PlanSummary; active: boolean; onOpen: () => void }) {
  const flags = [
    plan.blocking ? 'blocked' : null,
    plan.counts.conflicts ? `${plan.counts.conflicts} conflict${plan.counts.conflicts === 1 ? '' : 's'}` : null,
    plan.warnings ? `${plan.warnings} warning${plan.warnings === 1 ? '' : 's'}` : null,
  ].filter(Boolean)
  return (
    <ListItem
      className={`dir-ui-nav-row${active ? ' active' : ''}`}
      aria-current={active ? 'true' : undefined}
      onClick={onOpen}
      leading={
        <span className="dir-ui-nav-ico dir-ui-kit-pending-ico" aria-hidden>
          {plan.kind === 'update' ? <ArrowUpCircle /> : plan.kind === 'remove' ? <Trash2 /> : <Clock />}
        </span>
      }
      label={<span className="dir-ui-mono">{plan.kit}</span>}
      description={
        <span className="dir-ui-nav-fine">
          {planVerb(plan)}
          {flags.length ? ` · ${flags.join(' · ')}` : ''}
        </span>
      }
    />
  )
}

/** Screen A — the installed kits, pending reviews, and installing a new one. */
function Overview({
  host,
  listing,
  busy,
  checking,
  installing,
  onInstall,
  onCheck,
  onOpenKit,
  onOpenPlan,
  onReviewUpdate,
  onCancelInstall,
  narrow,
  onBack,
}: {
  host: Host
  listing: KitsListing | null
  busy: string | null
  checking: boolean
  installing: boolean
  onInstall: (ref: string) => void
  onCheck: () => void
  onOpenKit: (kit: string) => void
  onOpenPlan: (planId: string) => void
  onReviewUpdate: (kit: string) => void
  onCancelInstall: () => void
  narrow: boolean
  onBack: () => void
}) {
  const [ref, setRef] = useState('')
  void host
  const valid = /^[a-z0-9][a-z0-9-]*\/[a-z0-9][a-z0-9-]*(@\S+)?$/.test(ref.trim())
  const submitting = busy?.startsWith('install:') ?? false
  return (
    <div className="dir-ui-kit-screen">
      <header className="dir-ui-kit-head">
        <div className="dir-ui-kit-head-top">
          {narrow && installing ? (
            <Button variant="ghost" size="sm" className="dir-ui-kit-back" onClick={onBack}>
              Kits
            </Button>
          ) : null}
          <h2 className="dir-ui-kit-title">Kits</h2>
          <span className="dir-ui-kit-head-gap" />
          <Button variant="ghost" size="sm" onClick={onCheck} disabled={checking}>
            <RefreshCw aria-hidden className={checking ? uiClasses.spin : undefined} />
            {checking ? 'Checking…' : 'Check for updates'}
          </Button>
        </div>
        <p className="dir-ui-kit-desc">
          Agent profiles, skills and workers published together in the registry as <code>author/kit</code>. Installs and
          updates are reviewed here before anything is written.
        </p>
      </header>
      <div className="dir-ui-kit-scroll">
        {listing?.pending.map((p) => (
          <div key={p.plan_id} className="dir-ui-kit-banner">
            <Clock aria-hidden />
            <span className="dir-ui-kit-banner-text">
              {p.kind === 'install' ? 'An install' : p.kind === 'update' ? 'An update' : 'A removal'} is waiting for
              review:{' '}
              <strong>
                {p.kit} {p.kind === 'update' ? `${p.from} → ${p.to}` : (p.to ?? p.from)}
              </strong>
              {p.warnings ? (
                <span className="dir-ui-kit-fine">
                  {' '}
                  · {p.warnings} warning{p.warnings === 1 ? '' : 's'}
                </span>
              ) : null}
            </span>
            <Button variant="primary" size="sm" onClick={() => onOpenPlan(p.plan_id)}>
              Review
            </Button>
          </div>
        ))}

        <section className="dir-ui-kit-install" data-open={installing || undefined} aria-label="Install a kit">
          <Eyebrow as="div" className="dir-ui-kit-column-head">
            Install a kit
          </Eyebrow>
          <form
            className="dir-ui-kit-install-form"
            onSubmit={(e) => {
              e.preventDefault()
              if (valid) onInstall(ref.trim())
            }}
          >
            <Input
              value={ref}
              onChange={setRef}
              placeholder="author/kit or author/kit@1.2.0"
              aria-label="kit to install"
              autoFocus={installing}
              spellCheck={false}
            />
            <Button type="submit" variant="primary" size="sm" disabled={!valid || submitting}>
              {submitting ? 'Planning…' : 'Review install'}
            </Button>
            {installing ? (
              <Button type="button" variant="ghost" size="sm" onClick={onCancelInstall}>
                Cancel
              </Button>
            ) : null}
          </form>
          <p className="dir-ui-kit-fine">Nothing is written until you review the plan. From a terminal:</p>
          <TerminalCommandLine command="iii trigger directory::download-kit kit=<author>/<kit>" copy />
        </section>

        <section aria-label="Installed kits">
          <div className="dir-ui-kit-section-head">
            <Eyebrow as="div">Installed</Eyebrow>
            {listing?.updates_checked_at ? (
              <span className="dir-ui-kit-fine">checked {ago(listing.updates_checked_at)}</span>
            ) : null}
          </div>
          {listing?.updates_error ? (
            <p className="dir-ui-kit-fine">Last update check failed: {listing.updates_error}</p>
          ) : null}
          {listing === null ? (
            <div className="dir-ui-loading" aria-hidden>
              <Skeleton style={{ width: '70%' }} />
              <Skeleton style={{ width: '50%' }} />
            </div>
          ) : listing.kits.length === 0 ? (
            <EmptyState
              icon={Layers}
              title="No kits installed"
              description="Install one above, or ask an agent to call directory::download-kit — you review every install here."
              compact
            />
          ) : (
            <ul className="dir-ui-kit-cards">
              {listing.kits.map((k) => (
                <KitCard
                  key={k.kit}
                  kit={k}
                  planning={busy === k.kit}
                  onOpen={() => onOpenKit(k.kit)}
                  onReviewUpdate={() => onReviewUpdate(k.kit)}
                />
              ))}
            </ul>
          )}
          {listing ? <p className="dir-ui-kit-fine dir-ui-kit-lockpath">kits.lock · {listing.lock_path}</p> : null}
        </section>
      </div>
    </div>
  )
}

function KitCard({
  kit,
  planning,
  onOpen,
  onReviewUpdate,
}: {
  kit: InstalledKit
  planning: boolean
  onOpen: () => void
  onReviewUpdate: () => void
}) {
  const workers = Object.keys(kit.workers).length
  const facts = [
    `${kit.files.agents} profile${kit.files.agents === 1 ? '' : 's'}`,
    `${kit.files.skills} skill${kit.files.skills === 1 ? '' : 's'}`,
    `${workers} worker${workers === 1 ? '' : 's'}`,
  ]
  if (kit.files.edited) facts.push(`${kit.files.edited} file${kit.files.edited === 1 ? '' : 's'} edited by you`)
  if (kit.files.missing) facts.push(`${kit.files.missing} missing`)
  if (kit.files.skipped) facts.push(`${kit.files.skipped} kept yours`)
  const update = kit.update?.available
  return (
    <li className="dir-ui-kit-card">
      <button type="button" className="dir-ui-kit-card-main" onClick={onOpen}>
        <KitMark />
        <span className="dir-ui-kit-card-name">{kit.kit}</span>
        <span className="dir-ui-kit-version">{kit.version}</span>
        <span className="dir-ui-kit-card-status" data-update={update ? 'yes' : undefined}>
          {update ? `↑ ${update} available${kit.update?.major ? ' · major' : ''}` : kit.update ? 'up to date' : ''}
        </span>
        <span className="dir-ui-kit-card-facts">{facts.join(' · ')}</span>
      </button>
      {update ? (
        <Button variant="primary" size="sm" onClick={onReviewUpdate} disabled={planning}>
          {planning ? 'Planning…' : kit.pending.length ? 'Review update' : 'View update'}
        </Button>
      ) : null}
    </li>
  )
}

/** Loads a plan and picks the screen for its kind. */
function PlanScreen({
  host,
  planId,
  onBack,
  onRoute,
  onOpenEntry,
  confirmDiscard,
  notice,
  onNotice,
}: {
  host: Host
  planId: string
  onBack: () => void
  onRoute: (route: KitsRoute) => void
  onOpenEntry: (collection: 'agents' | 'skills', key: string) => void
  confirmDiscard: () => Promise<boolean>
  notice: string | null
  onNotice: (planId: string, message: string) => void
}) {
  const [record, setRecord] = useState<PlanRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    kitsApi(host)
      .plan(planId)
      .then((r) => {
        if (!cancelled) setRecord(r)
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e))
      })
    return () => {
      cancelled = true
    }
  }, [host, planId])

  if (error) {
    return (
      <div className="dir-ui-kit-screen">
        <div className="dir-ui-kit-scroll">
          <StatusPanel
            variant="warn"
            headline="This plan is no longer available."
            detail={error}
            action={
              <Button variant="ghost" size="sm" onClick={onBack}>
                Back to kits
              </Button>
            }
          />
        </div>
      </div>
    )
  }
  if (!record) {
    return (
      <div className="dir-ui-kit-screen">
        <div className="dir-ui-kit-scroll dir-ui-loading" aria-hidden>
          <Skeleton style={{ width: '50%' }} />
          <Skeleton style={{ width: '80%' }} />
          <Skeleton style={{ width: '65%' }} />
        </div>
      </div>
    )
  }
  const discard = async () => {
    if (!(await confirmDiscard())) return
    await kitsApi(host)
      .discard(planId)
      .catch(() => undefined)
    onBack()
  }
  const replanned = (plan: Plan, message: string) => {
    onNotice(plan.plan_id, message)
    onRoute({ view: 'plan', planId: plan.plan_id })
  }
  const common = {
    host,
    record,
    onBack,
    onDiscard: discard,
    onReplanned: replanned,
    onOpenProfile: (id: string) => onOpenEntry('agents', id),
    onOpenKit: (kit: string) => onRoute({ view: 'kit', kit }),
    notice,
  }
  switch (record.plan.kind) {
    case 'install':
      return <InstallReview {...common} />
    case 'update':
      return (
        <UpdateReview
          {...common}
          onIgnore={async () => {
            if (record.plan.to)
              await kitsApi(host)
                .ignore(record.plan.kit, record.plan.to)
                .catch(() => undefined)
            onBack()
          }}
        />
      )
    case 'remove':
      return <RemoveReview {...common} />
  }
}
