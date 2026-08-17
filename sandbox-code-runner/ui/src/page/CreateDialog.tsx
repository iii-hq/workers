/**
 * The create dialog: `sandbox::create` from a catalog image with the
 * optional knobs (name, cpus, memory, idle timeout, network, env). While
 * the create is in flight the elapsed counter drives honest stage notes
 * (requesting → pulling → still pulling); a failure renders the daemon's
 * whole S-code error (code, message, fix_note, retryable) instead of a
 * bare string.
 */

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  Input,
  Select,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { asRecord } from '../lib/payload'
import { parseSandboxError, type SandboxError } from './errors'
import type { CatalogImage } from './store'
import { type EnvRow, EnvRowsEditor, SandboxErrorCard } from './widgets'

/** Elapsed-based stage note — honest about what a slow create is doing. */
function createStageNote(elapsedSecs: number): string {
  if (elapsedSecs < 2) return 'requesting…'
  if (elapsedSecs < 45) return 'pulling image — cold pulls take 5–30s'
  return 'still pulling — large image or slow link'
}

const CREATE_EVENTS_FN = 'iii::sandbox-code-runner-ui::create-events'
const CREATE_EVENT_TRIGGER_TYPE = 'sandbox-code-runner::event'

/**
 * Create-phase stream — DORMANT today: the daemon does not emit
 * `{ kind: 'create_phase' }` yet (engine ask). The subscription is wired
 * so the steps list lights up the day it streams `pulling_image` /
 * `unpacking` / `booting_vm` / `ready`; bound only while a create is
 * actually pending.
 */
function useCreatePhases(host: Host, active: boolean): string[] {
  const [phases, setPhases] = useState<string[]>([])
  useEffect(() => {
    if (!active) {
      setPhases([])
      return
    }
    let offHandler: (() => void) | null = null
    let offTrigger: (() => void) | null = null
    try {
      offHandler = host.iii.on(CREATE_EVENTS_FN, (payload: unknown) => {
        const rec = asRecord(payload)
        if (rec?.kind !== 'create_phase') return
        const phase =
          typeof rec.phase === 'string' && rec.phase ? rec.phase : null
        if (!phase) return
        setPhases((prev) => (prev.includes(phase) ? prev : [...prev, phase]))
      })
      offTrigger = host.iii.registerTrigger({
        type: CREATE_EVENT_TRIGGER_TYPE,
        function_id: `${CREATE_EVENTS_FN}::${host.iii.browserId}`,
        config: {},
      })
    } catch {
      /* stream unavailable — the elapsed stages still tell the story */
    }
    return () => {
      offTrigger?.()
      offHandler?.()
    }
  }, [host, active])
  return phases
}

export function CreateDialog({
  host,
  open,
  images,
  onClose,
  onCreated,
}: {
  host: Host
  open: boolean
  images: CatalogImage[]
  onClose(): void
  onCreated(sandboxId: string | null): void
}) {
  const [image, setImage] = useState<string | undefined>(undefined)
  const [name, setName] = useState('')
  const [cpus, setCpus] = useState('')
  const [memoryMb, setMemoryMb] = useState('')
  const [idleSecs, setIdleSecs] = useState('')
  const [network, setNetwork] = useState<'off' | 'on'>('off')
  const [env, setEnv] = useState<EnvRow[]>([])
  const [pending, setPending] = useState(false)
  const [startedAt, setStartedAt] = useState<number | null>(null)
  const [elapsed, setElapsed] = useState(0)
  const [error, setError] = useState<SandboxError | null>(null)
  const phases = useCreatePhases(host, pending)

  // The elapsed counter — a create that is cold-pulling an image sits for
  // many seconds; a visibly ticking timer says "working", not "hung".
  useEffect(() => {
    if (startedAt === null) return
    const id = window.setInterval(
      () => setElapsed(Math.floor((Date.now() - startedAt) / 1000)),
      500,
    )
    return () => window.clearInterval(id)
  }, [startedAt])

  const imageOptions = images.map((img) => ({
    value: img.name,
    label: img.name,
    title: img.oci_ref || undefined,
  }))

  const numeric = (raw: string): number | undefined => {
    const n = Number(raw)
    return raw.trim() && Number.isFinite(n) && n > 0 ? n : undefined
  }

  const submit = () => {
    if (!image || pending) return
    const payload: Record<string, unknown> = { image }
    if (name.trim()) payload.name = name.trim()
    const cpusN = numeric(cpus)
    if (cpusN !== undefined) payload.cpus = cpusN
    const memN = numeric(memoryMb)
    if (memN !== undefined) payload.memory_mb = memN
    const idleN = numeric(idleSecs)
    if (idleN !== undefined) payload.idle_timeout_secs = idleN
    if (network === 'on') payload.network = true
    const envMap: Record<string, string> = {}
    for (const row of env) if (row.key.trim()) envMap[row.key.trim()] = row.value
    if (Object.keys(envMap).length > 0) payload.env = envMap

    setPending(true)
    setError(null)
    setStartedAt(Date.now())
    setElapsed(0)
    host.iii
      // Cold image pulls run long — give the call room to succeed.
      .trigger('sandbox::create', payload, { timeoutMs: 120_000 })
      .then((value) => {
        const id = asRecord(value)?.sandbox_id
        onCreated(typeof id === 'string' ? id : null)
        onClose()
      })
      .catch((err: unknown) => setError(parseSandboxError(err)))
      .finally(() => {
        setPending(false)
        setStartedAt(null)
      })
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && !pending && onClose()}>
      {!open ? null : (
        <DialogContent className="cr-page-dialog">
          <DialogTitle>new sandbox</DialogTitle>
          <DialogDescription>
            boots a microVM from a catalog image via sandbox::create.
          </DialogDescription>
          <div className="cr-page-dialog-fields">
            {imageOptions.length > 0 ? (
              <Select
                value={image}
                options={imageOptions}
                onChange={setImage}
                placeholder="image (from the catalog)"
                aria-label="image"
              />
            ) : (
              <Input
                value={image ?? ''}
                onChange={(next) => setImage(next || undefined)}
                placeholder="image name (catalog unavailable — type one)"
                preserveCase
                spellCheck={false}
                aria-label="image"
              />
            )}
            <Input
              value={name}
              onChange={setName}
              placeholder="name (optional)"
              preserveCase
              aria-label="name"
            />
            <div className="cr-page-field-row">
              <Input
                value={cpus}
                onChange={setCpus}
                placeholder="cpus"
                inputMode="numeric"
                aria-label="cpus"
              />
              <Input
                value={memoryMb}
                onChange={setMemoryMb}
                placeholder="memory_mb"
                inputMode="numeric"
                aria-label="memory in MiB"
              />
              <Input
                value={idleSecs}
                onChange={setIdleSecs}
                placeholder="idle_timeout_secs"
                inputMode="numeric"
                aria-label="idle timeout in seconds"
              />
            </div>
            <div className="cr-page-field-row cr-page-network-row">
              <Select
                value={network}
                options={[
                  { value: 'off', label: 'network: off (default)' },
                  {
                    value: 'on',
                    label: 'network: on',
                    title: 'outbound only — the guest can reach out; nothing can reach in',
                  },
                ]}
                onChange={setNetwork}
                aria-label="network"
              />
              {network === 'on' ? (
                <span className="cr-page-faint">outbound only</span>
              ) : null}
            </div>
            <EnvRowsEditor rows={env} onChange={setEnv} />
          </div>

          {pending ? (
            <div className="cr-page-pending" role="status">
              creating… {elapsed}s — {createStageNote(elapsed)}
              {phases.length > 0 ? (
                <ol className="cr-page-create-steps">
                  {phases.map((phase) => (
                    <li key={phase}>{phase.replace(/_/g, ' ')}</li>
                  ))}
                </ol>
              ) : null}
            </div>
          ) : null}
          {error ? <SandboxErrorCard error={error} /> : null}

          <div className="cr-page-dialog-actions">
            <Button variant="ghost" size="sm" onClick={onClose} disabled={pending}>
              cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={submit}
              disabled={pending || !image}
            >
              {pending ? 'creating…' : 'create'}
            </Button>
          </div>
        </DialogContent>
      )}
    </Dialog>
  )
}
