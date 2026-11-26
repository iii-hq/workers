import type { Host } from '@iii-dev/console-ui'
import { Button, Dialog, DialogContent, DialogDescription, DialogTitle, Skeleton } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import { envPageUrl, plainMessage, workerError } from './model'
import { LinkButton, Mono, Notice } from './parts'
import type { ConfigImported, ConfigSkipReason, Status, WorkerError } from './types'

const SKIP_WHY: Record<ConfigSkipReason, string> = {
  not_a_worker: 'no worker iii cloud runs reads it',
  not_a_plain_file: 'a link, or outside the repository',
  too_large: 'over 1 MiB',
  unreadable: "not a worker's config",
}
/** A worker's fields, one `dotted.key = value` line each: text as is, the rest as JSON. */
export const fieldLines = (fields: { key: string; value: unknown }[]) =>
  fields.map((f) => `${f.key} = ${typeof f.value === 'string' ? f.value : JSON.stringify(f.value)}`).join('\n')
/** The import's refusals: the worker's own sentences, which name the environment or the folder. */
const OWN_WORDS = new Set([
  'CONFIG_CHANGED',
  'CONFIG_DRAFT_PENDING',
  'ROLE_REQUIRED',
  'NOTHING_TO_IMPORT',
  'NO_LOCAL_CONFIG',
  'CONFIG_FOLDER_MISSING',
])
/** The `${NAME}` a left-out key is written as in iii cloud. */
const SECRET_REF = ['$', '{NAME}'].join('')

/**
 * O2: "Copy ./config to iii cloud". The dry run (`config::import {dry_run: true}`) previews one block per worker
 * (file → worker, its `dotted.key = value` fields) and what is left out; applying sends the dry run's version.
 * A version that moved (`CONFIG_CHANGED`) reads the preview again. `after`: what follows the copy ("and check again").
 */
export function ConfigImportDialog({
  host,
  status,
  envId,
  env,
  dir,
  after,
  onClose,
  onApplied,
}: {
  host: Host
  status: Status
  envId: string
  env: string
  /** The folder as the check names it, until the preview says it from the repository root. */
  dir: string
  after?: string
  onClose: () => void
  onApplied: () => void
}) {
  const [preview, setPreview] = useState<ConfigImported | null>(null)
  const [busy, setBusy] = useState<'preview' | 'apply' | null>('preview')
  const [error, setError] = useState<WorkerError | null>(null)
  const send = (dry_run: boolean, expected_version?: number) =>
    host.iii.call<ConfigImported>(
      'iii-cloud::config::import',
      { environment_id: envId, dry_run, ...(dry_run ? {} : { expected_version }) },
      { timeoutMs: 60_000 },
    )
  const fail = (err: unknown) => workerError(err, errorCode(err), errorMessage(err))
  const load = async () => {
    setBusy('preview')
    try {
      setPreview(await send(true))
    } catch (err) {
      setError(fail(err))
    } finally {
      setBusy(null)
    }
  }
  // The preview is read once, as the dialog opens.
  useEffect(() => {
    void load()
  }, [])
  const apply = async () => {
    if (!preview) return
    setBusy('apply')
    setError(null)
    try {
      await send(false, preview.version)
      onClose()
      onApplied()
    } catch (err) {
      const e = fail(err)
      setError(e)
      setBusy(null)
      // Someone changed the config since the preview: show what it is now.
      if (e.code === 'CONFIG_CHANGED') {
        setPreview(null)
        void load()
      }
    }
  }
  // A worker with only left-out keys still says which; only one with fields counts as copied.
  const workers = preview?.workers.filter((w) => w.fields.length || w.left_out.length || w.set_elsewhere.length) ?? []
  const n = workers.filter((w) => w.fields.length).length
  const url = envPageUrl(status, 'config')
  const where = preview?.directory ?? dir
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="iii-cloud-cf">
        <DialogTitle>{`Copy ${dir} to iii cloud`}</DialogTitle>
        <DialogDescription>
          {`What ${env}'s workers get from `}
          <Mono>{where}</Mono>
          {' on this machine. Until then they start with their defaults.'}
        </DialogDescription>
        {error ? (
          <Notice
            variant={error.code === 'CONFIG_CHANGED' || error.code === 'CONFIG_DRAFT_PENDING' ? 'warn' : 'alert'}
            headline={OWN_WORDS.has(error.code) && error.message ? error.message : plainMessage(error)}
            action={
              error.code === 'CONFIG_DRAFT_PENDING' && url ? (
                <LinkButton href={url}>Open in iii cloud</LinkButton>
              ) : null
            }
          />
        ) : null}
        {preview?.draft_changed ? (
          <Notice
            variant="warn"
            headline={`${env} has config edits in iii cloud that aren't applied yet`}
            detail="Apply or drop them there, then copy again."
            action={url ? <LinkButton href={url}>Open in iii cloud</LinkButton> : null}
          />
        ) : null}
        {busy === 'preview' && !preview ? (
          <Skeleton className="iii-cloud-set-skel" />
        ) : preview ? (
          <>
            {workers.length ? (
              <ul className="iii-cloud-ci-list" aria-label="What is copied">
                {workers.map((w) => (
                  <li key={`${w.file}:${w.worker}`}>
                    <span className="iii-cloud-ci-head">
                      <Mono>{w.file}</Mono>
                      <span aria-hidden>→</span>
                      <b>{w.worker}</b>
                    </span>
                    {w.fields.length ? <pre className="iii-cloud-ci-fields">{fieldLines(w.fields)}</pre> : null}
                    {w.left_out.map((k) => (
                      <p key={k} className="iii-cloud-ci-out">
                        <Mono>{k}</Mono>
                        {' left out: set it as a secret and write '}
                        <Mono>{SECRET_REF}</Mono>
                        {' in iii cloud'}
                      </p>
                    ))}
                    {w.set_elsewhere.length ? (
                      <p className="iii-cloud-ci-out">
                        {'Set by iii cloud: '}
                        <Mono>{w.set_elsewhere.join(', ')}</Mono>
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : null}
            {n ? null : (
              <p className="iii-cloud-desc">{`Nothing to copy: ${where} has no worker config iii cloud can take.`}</p>
            )}
            {preview.skipped.length ? (
              <ul className="iii-cloud-ci-skipped" aria-label="Not copied">
                {preview.skipped.map((x) => (
                  <li key={x.file}>
                    <Mono>{x.file}</Mono>
                    {` not copied: ${SKIP_WHY[x.reason] ?? x.reason}`}
                  </li>
                ))}
              </ul>
            ) : null}
          </>
        ) : null}
        <div className="iii-cloud-cf-foot">
          <p className="iii-cloud-desc">
            {`Secrets stay on this machine. ${env} gets it at once${after ? `, ${after}` : ''}.`}
          </p>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!n || !!busy || !!preview?.draft_changed} onClick={() => void apply()}>
            {preview ? `Copy ${n} ${n === 1 ? "worker's" : "workers'"} config to ${env}` : `Copy to ${env}`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
