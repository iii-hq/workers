import { Badge, Button, IconButton, Input, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCopyFlash, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { Check, Copy, RefreshCw } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import type { CloudTunnelStatus } from './cloud-model'
import { requestScope } from './model'

const STATUS_LABEL: Record<CloudTunnelStatus['status'], string> = {
  stopped: 'Stopped', starting: 'Starting', ready: 'Ready', reconnecting: 'Reconnecting', failed: 'Failed', unavailable: 'Unavailable',
}
type Snapshot = { revision: number; status: CloudTunnelStatus }

export function CloudTunnel({ host, webhookReady }: { host: Host; webhookReady: boolean }) {
  const instance = useId()
  const revision = useRef(0)
  const scope = useRef(requestScope())
  const busy = useRef(false)
  const [pending, setPending] = useState<'start' | 'stop' | null>(null)
  const [actionError, setActionError] = useState<{ action: 'start' | 'stop'; message: string } | null>(null)
  const [actionStatus, setActionStatus] = useState<CloudTunnelStatus | null>(null)
  const live = useWorkerLive<Snapshot>({ iii: host.iii, triggers: [], pollMs: 4000,
    handlerId: `iii::whatsapp-ui::cloud-tunnel::${instance}`,
    fetch: async () => {
      const current = revision.current
      const status = await host.iii.trigger<CloudTunnelStatus>('whatsapp::cloud::tunnel::status', {})
      if (current === revision.current && !busy.current) {
        setActionError((previous) => previous && (previous.action === 'start' ? status.enabled : !status.enabled) ? null : previous)
      }
      return { revision: current, status }
    },
  })
  useEffect(() => () => scope.current.invalidate(), [])

  // A poll started before or during a mutation cannot restore the previous URL.
  const fresh = live.data?.revision === revision.current && !pending ? live.data.status : null
  const known = fresh ?? actionStatus ?? live.data?.status
  const webhookUrl = !live.error && fresh?.enabled && fresh.status === 'ready' && !fresh.error && webhookReady ? fresh.webhook_url : null
  const statusLabel = pending ? pending === 'start' ? 'Starting…' : 'Stopping…'
    : live.error ? 'Unavailable' : fresh ? STATUS_LABEL[fresh.status] : 'Checking…'
  const retry = known?.enabled && (known.status === 'failed' || known.status === 'unavailable')
  const canStart = webhookReady && !!known && !pending && (!known.enabled || retry)
  const problem = actionError?.message ?? live.error ?? fresh?.error

  const change = async (action: 'start' | 'stop') => {
    if (busy.current || (action === 'start' && !webhookReady)) return
    busy.current = true
    revision.current += 1
    const current = scope.current.begin()
    setPending(action); setActionError(null)
    try {
      const result = await host.iii.trigger<CloudTunnelStatus>(`whatsapp::cloud::tunnel::${action}`, {})
      if (current()) setActionStatus(result)
    } catch (cause) { if (current()) setActionError({ action, message: errorMessage(cause) }) }
    finally {
      if (current()) {
        revision.current += 1
        busy.current = false
        setPending(null)
        live.refresh()
      }
    }
  }

  return <section className="wa-ui-cloud-tunnel" aria-label="Development webhook">
    <div className="wa-ui-cloud-tunnel-heading">
      <div className="wa-ui-heading-copy"><h3 className="wa-ui-title">Development webhook</h3><p className="wa-ui-note">Create a public HTTPS callback with Quick Tunnel.</p></div>
      <span role="status"><Badge variant={webhookUrl ? 'ok' : known?.status === 'failed' || live.error ? 'warn' : 'default'}>{statusLabel}</Badge></span>
      <IconButton label="Refresh development tunnel" disabled={!!pending} onClick={live.refresh}><RefreshCw size={16} /></IconButton>
    </div>
    {webhookUrl ? <WebhookUrl key={webhookUrl} url={webhookUrl} /> : <p className="wa-ui-note">
      {pending === 'stop' ? 'Releasing this worker’s development tunnel.'
        : pending === 'start' || fresh?.status === 'starting' ? 'Waiting for a public URL. This updates automatically.'
        : fresh?.status === 'reconnecting' ? 'The tunnel is reconnecting. Wait for the current callback URL before updating Meta.'
        : !webhookReady ? 'Enable the Cloud API and set the app secret and webhook verify token in Configure worker before starting.'
        : !fresh || live.error ? 'Refresh the tunnel status to check its current callback URL.'
        : 'Start a development tunnel to generate the full callback URL.'}
    </p>}
    {problem && <StatusPanel role="alert" variant="alert" headline="Development tunnel needs attention" detail={problem} action={<Button variant="ghost" disabled={!!pending} onClick={() => { setActionError(null); live.refresh() }}>Refresh status</Button>} />}
    <div className="wa-ui-cloud-tunnel-actions">
      {(!known?.enabled || retry) && <Button variant="ghost" disabled={!canStart} onClick={() => void change('start')}>{pending === 'start' ? 'Starting…' : retry ? 'Retry development tunnel' : 'Start development tunnel'}</Button>}
      {known?.enabled && <Button variant="ghost" disabled={!!pending} onClick={() => void change('stop')}>{pending === 'stop' ? 'Stopping…' : 'Stop tunnel'}</Button>}
    </div>
    <p className="wa-ui-note">In Meta, paste the callback URL, use your configured verify token, and subscribe to messages. This URL is temporary: update Meta whenever it changes. Starting a tunnel does not register it with Meta.</p>
  </section>
}

function WebhookUrl({ url }: { url: string }) {
  const id = useId()
  const copy = useCopyFlash(url)
  return <div className="wa-ui-cloud-tunnel-url">
    <label htmlFor={id} className="wa-ui-cloud-tunnel-label">Callback URL</label>
    <div className="wa-ui-cloud-tunnel-url-row"><Input id={id} readOnly value={url} onChange={() => {}} className="wa-ui-mono" />
      <Button variant="ghost" onClick={copy.copy}>{copy.state === 'copied' ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}{copy.state === 'copied' ? 'Copied' : 'Copy webhook URL'}</Button>
    </div>
    {copy.state === 'failed' && <p className="wa-ui-note" role="status">Could not copy the URL. Select it above and copy it manually.</p>}
  </div>
}
