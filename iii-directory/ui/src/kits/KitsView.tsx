/**
 * The Kits section of the Directory page, on the same frame as Agents and
 * Skills: the library column lists plans waiting for review, then installed
 * kits; the document shows one installed kit (screen B), a plan under
 * review (C install, D update, F removal), or — with nothing open — what
 * kits are and the one way in, "Install kit". Everything refreshes on
 * `directory::kits::on-change`.
 */

import {
  Button,
  type Host,
  PageSidebar,
  ShadcnButton,
  ShadcnDialog,
  ShadcnDialogContent,
  ShadcnDialogDescription,
  ShadcnDialogFooter,
  ShadcnDialogHeader,
  ShadcnDialogTitle,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuTrigger,
  ShadcnInput,
  ShadcnTooltip,
  ShadcnTooltipContent,
  ShadcnTooltipTrigger,
  Skeleton,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import { ArrowUpCircle, Clock, Ellipsis, Plus, Puzzle, RefreshCw, Trash2 } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from 'react'
import { ago } from '../lib/format'
import { type BrowserRow, CopyGlyph } from '../page/browser'
import { Library } from '../page/library'
import { countLabel } from '../page/library-model'
import { errorText, kitsApi, type PlanRecord, useKitsChange } from './api'
import { InstallReview } from './InstallReview'
import { KitDetail } from './KitDetail'
import { RemoveReview } from './RemoveReview'
import type { KitsListing, Plan, PlanSummary } from './types'
import { UpdateReview } from './UpdateReview'

/** Same drill-in width as the Agents and Skills browser. */
const NARROW_BELOW = 760

const KIT_REF = /^[a-z0-9][a-z0-9-]*\/[a-z0-9][a-z0-9-]*(@\S+)?$/

const CLI_COMMAND = 'iii call directory::download-kit kit=<author>/<kit>'

export type KitsRoute =
  | { view: 'home' }
  | { view: 'kit'; kit: string }
  | { view: 'plan'; planId: string }
  | { view: 'install' }

/** What a pending plan would do, in words: "Install 1.2.0", "Update 1.1 → 1.2", "Remove 1.0". */
function planVerb(p: PlanSummary): string {
  if (p.kind === 'install') return `Install ${p.to ?? ''}`
  if (p.kind === 'update') return `Update ${p.from ?? ''} → ${p.to ?? ''}`
  return `Remove ${p.from ?? ''}`
}

/** Library rows: plans waiting for a decision first, then installed kits. */
export function kitRows(listing: KitsListing): BrowserRow[] {
  const pending = listing.pending.map((p): BrowserRow => {
    const flags = [
      p.blocking ? 'blocked' : null,
      p.counts.conflicts ? countLabel(p.counts.conflicts, { one: 'conflict', other: 'conflicts' }) : null,
      p.warnings ? countLabel(p.warnings, { one: 'warning', other: 'warnings' }) : null,
    ].filter(Boolean)
    return {
      key: `plan:${p.plan_id}`,
      icon:
        p.kind === 'update' ? (
          <ArrowUpCircle aria-hidden />
        ) : p.kind === 'remove' ? (
          <Trash2 aria-hidden />
        ) : (
          <Clock aria-hidden />
        ),
      title: p.kit,
      description: [planVerb(p).trim(), ...flags].join(' · '),
      fine: '',
      group: 'review',
      groupLabel: 'Needs review',
      badge: 'review',
    }
  })
  const kits = listing.kits.map((k): BrowserRow => {
    const workers = Object.keys(k.workers).length
    const facts = [
      k.version,
      countLabel(k.files.agents, { one: 'agent', other: 'agents' }),
      countLabel(k.files.skills, { one: 'skill', other: 'skills' }),
      countLabel(workers, { one: 'worker', other: 'workers' }),
    ]
    return {
      key: `kit:${k.kit}`,
      icon: <Puzzle aria-hidden />,
      title: k.kit,
      description: facts.join(' · '),
      fine: '',
      group: 'installed',
      groupLabel: 'Installed',
      ...(k.update?.available ? { badge: 'update' } : k.files.edited ? { badge: 'edited' } : {}),
    }
  })
  return [...pending, ...kits]
}

/** The Kits section: the library of plans and installed kits beside the open kit, plan or home. */
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
  const [query, setQuery] = useState('')
  const [collapsed, setCollapsed] = usePaneState<string[]>('iii-directory:kits:collapsed', [])
  const [installOpen, setInstallOpen] = useState(false)
  // A replanned apply lands on a new plan id; its reason rides along.
  const [notices, setNotices] = useState<Record<string, string>>({})
  const { ref, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const { confirm, dialog } = useConfirm()
  const searchRef = useRef<HTMLInputElement | null>(null)
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

  // `{ view: 'install' }` (a chat card, the palette) opens the dialog.
  useEffect(() => {
    if (route.view !== 'install') return
    setInstallOpen(true)
    onRoute({ view: 'home' })
  }, [route, onRoute])

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
      setActionError(`Unable to check for updates: ${errorText(e)}`)
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

  const selectedKey = route.view === 'kit' ? `kit:${route.kit}` : route.view === 'plan' ? `plan:${route.planId}` : null
  const openRow = (key: string) => {
    if (key.startsWith('plan:')) onRoute({ view: 'plan', planId: key.slice(5) })
    else if (key.startsWith('kit:')) onRoute({ view: 'kit', kit: key.slice(4) })
  }

  const showSide = !narrow || route.view === 'home'
  const showMain = !narrow || route.view !== 'home'

  const sectionMenu = (
    <ShadcnDropdownMenu>
      <ShadcnDropdownMenuTrigger
        render={
          <ShadcnButton variant="ghost" size="icon-sm" aria-label="More kit actions">
            <Ellipsis aria-hidden />
          </ShadcnButton>
        }
      />
      <ShadcnDropdownMenuContent align="end" sideOffset={6} className="dir-menu">
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuItem disabled={checking} onClick={() => void checkUpdates()}>
            <RefreshCw aria-hidden />
            {checking ? 'Checking for updates…' : 'Check for updates'}
          </ShadcnDropdownMenuItem>
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )

  return (
    <div className={`dir-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`} ref={ref}>
      {dialog}
      <InstallDialog
        open={installOpen}
        onOpenChange={setInstallOpen}
        install={async (kitRef) => {
          const out = await api.download(kitRef)
          refresh()
          if (!out.plan) throw new Error(out.message)
          setInstallOpen(false)
          onRoute({ view: 'plan', planId: out.plan.plan_id })
        }}
      />
      {showSide ? (
        <PageSidebar
          label="kit list"
          side={panelSide}
          collapsible
          storageKey="iii-directory:library"
          defaultWidth={320}
          minWidth={260}
          narrow={narrow}
          className="dir-side"
          header={nav}
          collapsedActions={
            <ShadcnButton variant="ghost" size="icon-sm" aria-label="Install kit" onClick={() => setInstallOpen(true)}>
              <Plus aria-hidden />
            </ShadcnButton>
          }
        >
          <Library
            noun="kit"
            nounPlural="Kits"
            rows={listing ? kitRows(listing) : null}
            error={listError}
            onRetry={refresh}
            selected={selectedKey}
            onOpen={openRow}
            query={query}
            onQuery={setQuery}
            searchRef={searchRef}
            onCreate={() => setInstallOpen(true)}
            createLabel="Install kit"
            menu={sectionMenu}
            groupOrder={['review', 'installed']}
            collapsed={collapsed}
            onToggleGroup={(id) =>
              setCollapsed((current) => (current.includes(id) ? current.filter((g) => g !== id) : [...current, id]))
            }
            emptyTitle="No kits yet"
            emptyBody="Install one to add agents, skills and workers in one reviewed step."
          />
        </PageSidebar>
      ) : null}
      {showMain ? (
        <section className="dir-doc dir-kit-doc" aria-label="Kit">
          {actionError ? (
            <div className="dir-notice" role="alert" data-tone="alert">
              <span>{actionError}</span>
              <span className="dir-notice-actions">
                <ShadcnButton variant="ghost" size="sm" onClick={() => setActionError(null)}>
                  Dismiss
                </ShadcnButton>
              </span>
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
                  description: 'Nothing was written, and you can plan it again any time.',
                  confirmLabel: 'Discard plan',
                  cancelLabel: 'Keep reviewing',
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
            <KitsHome listing={listing} onInstall={() => setInstallOpen(true)} />
          )}
        </section>
      ) : null}
    </div>
  )
}

/** Nothing open: what kits are, the one way in, and — out of the way — the
 * terminal route and where the lock file lives. */
function KitsHome({ listing, onInstall }: { listing: KitsListing | null; onInstall: () => void }) {
  return (
    <div className="dir-doc-empty">
      <div className="dir-doc-empty-card">
        <h2 className="dir-doc-empty-title">Kits install agents, skills and workers together</h2>
        <p className="dir-doc-empty-body">
          A kit is published to the registry as <span className="dir-mono">author/kit</span>. Every install and update
          is reviewed here before anything is written.
        </p>
        <div className="dir-doc-empty-actions">
          <ShadcnButton size="sm" onClick={onInstall}>
            <Plus aria-hidden />
            Install kit
          </ShadcnButton>
        </div>
        {listing?.updates_error ? (
          <p className="dir-doc-empty-fine" data-tone="alert">
            The last update check failed: {listing.updates_error}
          </p>
        ) : listing?.updates_checked_at ? (
          <p className="dir-doc-empty-fine">Updates checked {ago(listing.updates_checked_at)}</p>
        ) : null}
        <details className="dir-dev">
          <summary>For developers</summary>
          <div className="dir-dev-body">
            <p className="dir-dev-label">Install from a terminal</p>
            <CopyLine value={CLI_COMMAND} label="Copy command" />
            {listing ? (
              <>
                <p className="dir-dev-label">Lock file</p>
                <CopyLine value={listing.lock_path} label="Copy path" />
              </>
            ) : null}
          </div>
        </details>
      </div>
    </div>
  )
}

/** A mono value with a copy button (copy ⇄ copied cross-fade). */
function CopyLine({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const copy = () =>
    void copyText(value).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => setCopied(false), 1400)
    })
  return (
    <div className="dir-dev-line">
      <code className="dir-mono">{value}</code>
      <ShadcnTooltip>
        <ShadcnTooltipTrigger
          render={
            <ShadcnButton variant="ghost" size="icon-sm" aria-label={copied ? 'Copied' : label} onClick={copy}>
              <CopyGlyph copied={copied} />
            </ShadcnButton>
          }
        />
        <ShadcnTooltipContent>{copied ? 'Copied' : label}</ShadcnTooltipContent>
      </ShadcnTooltip>
    </div>
  )
}

/** "Install kit": one field, one primary. Nothing is written here — the
 * button asks the worker for a plan and opens it for review. */
function InstallDialog({
  open,
  onOpenChange,
  install,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  install: (kitRef: string) => Promise<void>
}) {
  const [value, setValue] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const fieldId = useId()
  // `submitting` is NOT reset here: it clears only when the request settles,
  // and the dialog refuses to close until then (below), so a cancelled
  // dialog can never jump to a plan or start a second download.
  useEffect(() => {
    if (!open) setError(null)
  }, [open])
  const kitRef = value.trim()
  const valid = KIT_REF.test(kitRef)
  const submit = async () => {
    if (!valid || submitting) return
    setSubmitting(true)
    setError(null)
    try {
      await install(kitRef)
      setValue('')
    } catch (e) {
      setError(`Unable to plan the install: ${errorText(e).replace(/[.\s]+$/, '')}.`)
    } finally {
      setSubmitting(false)
    }
  }
  return (
    <ShadcnDialog
      open={open}
      onOpenChange={(next) => {
        // Escape, the corner ✕ and an outside click wait for the request.
        if (next || !submitting) onOpenChange(next)
      }}
    >
      <ShadcnDialogContent>
        <form
          onSubmit={(e) => {
            e.preventDefault()
            void submit()
          }}
        >
          <ShadcnDialogHeader>
            <ShadcnDialogTitle>Install kit</ShadcnDialogTitle>
            <ShadcnDialogDescription>Nothing is written until you review the plan.</ShadcnDialogDescription>
          </ShadcnDialogHeader>
          <div className="dir-install-field">
            <label htmlFor={fieldId} className="dir-field-label">
              Kit
            </label>
            <ShadcnInput
              id={fieldId}
              className="dir-mono"
              value={value}
              onChange={(e) => setValue(e.currentTarget.value)}
              placeholder="acme/support@1.2.0"
              spellCheck={false}
              autoComplete="off"
              readOnly={submitting}
              aria-invalid={error ? true : undefined}
              aria-describedby={`${fieldId}-help`}
            />
            <p id={`${fieldId}-help`} className={error ? 'dir-field-help dir-field-error' : 'dir-field-help'}>
              {error ?? 'author/kit, with an optional @version.'}
            </p>
          </div>
          <ShadcnDialogFooter>
            <ShadcnButton type="button" variant="ghost" disabled={submitting} onClick={() => onOpenChange(false)}>
              Cancel
            </ShadcnButton>
            <ShadcnButton type="submit" disabled={!valid || submitting}>
              {submitting ? 'Planning…' : 'Review install'}
            </ShadcnButton>
          </ShadcnDialogFooter>
        </form>
      </ShadcnDialogContent>
    </ShadcnDialog>
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
