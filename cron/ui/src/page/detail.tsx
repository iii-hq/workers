import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  JsonHighlight,
  StatusDot,
} from '@iii-dev/console-ui'
import { useState } from 'react'
import type { SessionCronTask, SystemCronObserver } from '../lib/api'
import { describeCron, nextCronRun, untilLabel } from '../lib/cron'
import { cadenceLabel, observerStateView } from './rows'
import { statusView } from './status'

function formatTimestamp(value: number): string {
  return `${new Date(value).toISOString().replace('T', ' ').slice(0, 19)} UTC`
}

function DetailRow({ label, children, mono = false }: { label: string; children: React.ReactNode; mono?: boolean }) {
  return (
    <div className="cron-ui-detail-row">
      <dt className="cron-ui-detail-row-label">{label}</dt>
      <dd className={mono ? 'cron-ui-detail-row-value mono' : 'cron-ui-detail-row-value'}>{children}</dd>
    </div>
  )
}

export function TaskDetail({
  task,
  now,
  onReplace,
  onRemove,
  onCopyId,
  onOpenConversation,
}: {
  task: SessionCronTask
  now: Date
  onReplace: () => void
  onRemove: () => void
  onCopyId: () => void
  onOpenConversation: () => void
}) {
  const [confirming, setConfirming] = useState(false)
  const view = statusView(task, now.getTime())
  const next = nextCronRun(task.expression, now)
  const payload = task.conditions.length > 0 ? task.conditions : null
  return (
    <div className="cron-ui-detail">
      <header className="cron-ui-detail-head">
        <h2 className="cron-ui-detail-title">{task.label ?? 'Untitled schedule'}</h2>
        <span className="cron-ui-status">
          <StatusDot tone={view.tone} aria-hidden />
          <span>{view.label}</span>
        </span>
      </header>

      <dl className="cron-ui-fields">
        <DetailRow label="Next run">
          {next ? `${formatTimestamp(next.getTime())} · ${untilLabel(next, now)}` : 'Not scheduled'}
        </DetailRow>
        <DetailRow label="Cadence">{cadenceLabel(task.expression)}</DetailRow>
        <DetailRow label="Cron expression" mono>
          {task.expression}
        </DetailRow>
        <DetailRow label="Timezone">UTC</DetailRow>
        <DetailRow label="Target" mono>
          {task.target ?? 'wake'}
        </DetailRow>
        <DetailRow label="Wakes">{task.sessionTitle ?? 'its own session'}</DetailRow>
        <DetailRow label="Session" mono>
          {task.sessionId}
        </DetailRow>
        <DetailRow label="Runs">
          {task.once
            ? `${task.fires} of 1 (runs once)`
            : task.maxFires !== undefined
              ? `${task.fires} of ${task.maxFires}`
              : `${task.fires}`}
        </DetailRow>
        {task.expiresAt !== undefined ? <DetailRow label="Expires">{formatTimestamp(task.expiresAt)}</DetailRow> : null}
        <DetailRow label="Created">{formatTimestamp(task.createdAt)}</DetailRow>
        <DetailRow label="Observer" mono>
          {task.observerId}
        </DetailRow>
      </dl>

      {payload ? (
        <section className="cron-ui-detail-section">
          <h3 className="cron-ui-detail-subtitle">Conditions</h3>
          <JsonHighlight code={JSON.stringify(payload, null, 2)} wrap />
        </section>
      ) : null}

      <footer className="cron-ui-detail-actions">
        <Button variant="ghost" onClick={onOpenConversation}>
          Open conversation
        </Button>
        <Button variant="ghost" onClick={onCopyId}>
          Copy id
        </Button>
        <Button variant="ghost" onClick={onReplace}>
          Replace…
        </Button>
        <Button variant="primary" onClick={() => setConfirming(true)}>
          Remove
        </Button>
      </footer>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogTitle>Remove this schedule?</DialogTitle>
          <DialogDescription>
            {task.label ?? task.observerId} stops firing immediately. This cannot be undone; the schedule has to be
            created again.
          </DialogDescription>
          <div className="cron-ui-detail-actions">
            <DialogClose asChild>
              <Button variant="ghost">Keep it</Button>
            </DialogClose>
            <Button
              variant="primary"
              onClick={() => {
                setConfirming(false)
                onRemove()
              }}
            >
              Remove
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}

export function ObserverDetail({ observer, now }: { observer: SystemCronObserver; now: Date }) {
  const next = nextCronRun(observer.expression, now)
  const view = observerStateView(observer.state)
  return (
    <div className="cron-ui-detail">
      <header className="cron-ui-detail-head">
        <h2 className="cron-ui-detail-title">{observer.functionId}</h2>
        <span className="cron-ui-status">
          <StatusDot tone={view.tone} aria-hidden />
          <span>{view.label}</span>
        </span>
      </header>

      <dl className="cron-ui-fields">
        <DetailRow label="Owner">{observer.workerName}</DetailRow>
        <DetailRow label="Next run">
          {next ? `${formatTimestamp(next.getTime())} · ${untilLabel(next, now)}` : 'Not scheduled'}
        </DetailRow>
        <DetailRow label="Cadence">{describeCron(observer.expression) ?? observer.expression}</DetailRow>
        <DetailRow label="Cron expression" mono>
          {observer.expression}
        </DetailRow>
        <DetailRow label="Timezone">UTC</DetailRow>
        <DetailRow label="Runs">{observer.fires}</DetailRow>
        <DetailRow label="Observer" mono>
          {observer.id}
        </DetailRow>
      </dl>

      {observer.filters ? (
        <section className="cron-ui-detail-section">
          <h3 className="cron-ui-detail-subtitle">Filters</h3>
          <JsonHighlight code={JSON.stringify(observer.filters, null, 2)} wrap />
        </section>
      ) : null}

      <p className="cron-ui-detail-note">
        This observer belongs to the {observer.workerName} worker. It is shown so the schedule is visible in one place;
        change it where the worker declares it.
      </p>
    </div>
  )
}
