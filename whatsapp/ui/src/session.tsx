import { Badge, Button, EmptyState, Input, List, ListItem, LiveRegion, SettingsField, SettingsList, Skeleton, StatusPanel, type Host, type PageCommandsApi, type LiveAnnouncement } from '@iii-dev/console-ui'
import { errorMessage, formatDuration, formatRelative } from '@iii-dev/console-ui/format'
import { useEffect, useId, useRef, useState } from 'react'
import { ChevronDown, MessageCircle, Play, Square } from 'lucide-react'
import QRCode from 'qrcode'
import { eventDetail, pairingIsCurrent, pairingPhone, requestScope, STATUS_LABEL, type PairingCode, type SessionStatus } from './model'
import { useSession } from './use-session'

export function SessionBadge({ status }: { status: SessionStatus }) {
  return <Badge variant={status === 'connected' ? 'ok' : status === 'failed' ? 'alert' : status === 'pairing' || status === 'starting' || status === 'stopping' ? 'warn' : 'default'}>{STATUS_LABEL[status]}</Badge>
}

function PairingQr({ pair }: { pair: PairingCode }) {
  const [image, setImage] = useState<{ code: string; url: string } | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let active = true
    setImage(null)
    setError(null)
    void QRCode.toDataURL(pair.code, { errorCorrectionLevel: 'M', margin: 4, width: 288 }).then(
      (url) => { if (active) setImage({ code: pair.code, url }) },
      (cause) => { if (active) setError(errorMessage(cause)) },
    )
    return () => { active = false }
  }, [pair.code])
  if (error) return <StatusPanel variant="alert" headline="Could not generate the QR code" detail={error} />
  return image?.code === pair.code
    ? <img className="wa-ui-qr" src={image.url} width={288} height={288} alt="Pairing QR code for the selected WhatsApp session" />
    : <Skeleton className="wa-ui-qr" aria-label="Preparing the pairing QR code" />
}

const EVENT_LABELS: Record<string, string> = {
  'message.received': 'Message received', 'message.receipt': 'Message receipt',
  'session.created': 'Session created', 'session.starting': 'Session starting',
  'session.connected': 'Session connected', 'session.disconnected': 'Session disconnected',
  'session.stopped': 'Session stopped', 'session.stopping': 'Session stopping',
  'session.qr': 'QR code available', 'session.qr_expired': 'QR code expired', 'session.pair_code': 'Pairing code available',
  'session.pair_code_invalidated': 'Pairing code invalidated', 'session.paired': 'Account linked',
  'session.pairing_failed': 'Pairing failed', 'session.logged_out': 'Account unlinked',
}

const CONNECTION_COPY: Record<SessionStatus, { title: string; detail: string }> = {
  stopped: { title: 'Session stopped', detail: 'Start to connect. Saved credentials are reused when available.' },
  starting: { title: 'Connecting', detail: 'Keep your phone nearby. Pairing options appear below if needed.' },
  pairing: { title: 'Link your account', detail: 'Scan the QR code or request a pairing code for your phone number.' },
  connected: { title: 'Account connected', detail: 'Open Messages for available conversations or Groups for joined groups.' },
  disconnected: { title: 'Session disconnected', detail: 'Start to reconnect. Your available message archive remains readable while offline.' },
  stopping: { title: 'Stopping session', detail: 'Saved credentials will remain available for the next connection.' },
  logged_out: { title: 'Account unlinked', detail: 'Start the session, then pair your phone to reconnect.' },
  failed: { title: 'Connection failed', detail: 'Check the error and recent activity before starting the session again.' },
}

const CONNECTION_TIPS = [
  { title: 'Find Linked devices', text: 'In WhatsApp, open Settings on iPhone or the menu on Android. Select Linked devices, then Link a device.' },
  { title: 'Reuse a saved session', text: 'Stopping keeps saved credentials. Start the same session to reconnect instead of creating another one.' },
  { title: 'Wait for available history', text: 'Messages shows captured conversations and history shared by WhatsApp. Older history requested from a conversation may arrive later.' },
]

function ConnectionHelp() {
  return <details className="wa-ui-connection-help">
    <summary><span>Connection help</span><ChevronDown size={16} aria-hidden="true" /></summary>
    <div className="wa-ui-help-items">{CONNECTION_TIPS.map((tip) => <div key={tip.title}>
      <h4 className="wa-ui-title">{tip.title}</h4>
      <p className="wa-ui-note">{tip.text}</p>
    </div>)}</div>
  </details>
}

export function SessionDetail({ host, sessionId, refreshList, commands }: {
  host: Host; sessionId: string; refreshList(): void; commands?: PageCommandsApi
}) {
  const [operation, setOperation] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const busy = useRef(false)
  const scope = useRef(requestScope())
  const pairScope = useRef(requestScope())
  const { session, qr, feed, error, loading, refresh } = useSession(host, sessionId, operation === 'start' || operation === 'stop')
  const [phone, setPhone] = useState('')
  const [phonePair, setPhonePair] = useState<PairingCode | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const [now, setNow] = useState(Date.now())
  const instance = useId()
  const status = session?.status ?? 'stopped'
  const canStart = !!session && !error && ['stopped', 'disconnected', 'logged_out', 'failed'].includes(status)
  const canStop = !!session && !['stopped', 'logged_out', 'stopping'].includes(status)
  const pairAllowed = status === 'pairing' && !error && operation !== 'start' && operation !== 'stop'
  const validQr = pairAllowed && pairingIsCurrent(qr, status, now) ? qr : null
  const validPhonePair = pairAllowed && pairingIsCurrent(phonePair, status, now) ? phonePair : null

  useEffect(() => {
    const tick = () => setNow(Date.now())
    const timer = window.setInterval(tick, 1000)
    document.addEventListener('visibilitychange', tick)
    return () => { window.clearInterval(timer); document.removeEventListener('visibilitychange', tick); scope.current.invalidate(); pairScope.current.invalidate() }
  }, [])
  const lastPairInvalidation = feed.events.filter((event) => ['session.pair_code_invalidated', 'session.paired', 'session.pairing_failed'].includes(event.kind)).at(-1)?.sequence
  useEffect(() => {
    pairScope.current.invalidate()
    setPhonePair(null)
  }, [status, error, lastPairInvalidation, session?.created_at_ms])
  useEffect(() => {
    if (phonePair && phonePair.expires_at_ms <= now) setPhonePair(null)
  }, [phonePair, now])

  const lifecycle = async (action: 'start' | 'stop') => {
    if (busy.current || (action === 'start' ? !canStart : !canStop)) return
    busy.current = true
    const current = scope.current.begin()
    pairScope.current.invalidate()
    setPhonePair(null)
    setPhone('')
    setOperation(action)
    setActionError(null)
    try {
      await host.iii.trigger(`whatsapp::sessions::${action}`, { session_id: sessionId }, { timeoutMs: 730000 })
      if (!current()) return
      setAnnouncement({ seq: Date.now(), text: action === 'start' ? 'Session start requested.' : 'Session stopped; credentials preserved.', urgency: 'polite' })
      refreshList()
    } catch (cause) { if (current()) setActionError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setOperation(null); refresh() } }
  }

  const requestCode = async () => {
    if (busy.current || !pairAllowed) return
    let number: string
    try { number = pairingPhone(phone) } catch (cause) { setActionError(errorMessage(cause)); return }
    busy.current = true
    const current = scope.current.begin()
    const samePhone = pairScope.current.begin()
    setOperation('pair')
    setActionError(null)
    setPhonePair(null)
    try {
      const result = await host.iii.trigger<PairingCode>('whatsapp::sessions::pair_code', { session_id: sessionId, phone_number: number }, { timeoutMs: 610000 })
      if (current() && samePhone() && result.expires_at_ms > Date.now()) setPhonePair(result)
    } catch (cause) { if (current() && samePhone()) setActionError(errorMessage(cause)) }
    finally { if (current()) { busy.current = false; setOperation(null); refresh() } }
  }

  useEffect(() => commands?.register([
    { id: 'start', title: 'Start session', shortcut: 'S', enabled: () => canStart && !busy.current, run: () => void lifecycle('start') },
    { id: 'stop', title: 'Stop session', shortcut: 'X', enabled: () => canStop && !busy.current, run: () => void lifecycle('stop') },
    { id: 'pair-code', title: 'Request pairing code', enabled: () => pairAllowed && !!phone.trim() && !busy.current, run: () => void requestCode() },
  ]), [commands, canStart, canStop, pairAllowed, phone])

  if (!session && loading) return <div className="wa-ui-content wa-ui-connection"><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></div>
  return <div className="wa-ui-content wa-ui-connection">
    {error && <StatusPanel variant="alert" headline="Could not refresh this session" detail={error} action={<Button variant="ghost" onClick={refresh}>Try again</Button>} />}
    {actionError && <StatusPanel variant="alert" headline="The operation did not complete" detail={actionError} action={<Button variant="ghost" onClick={() => setActionError(null)}>Dismiss</Button>} />}
    <div className="wa-ui-connection-grid">
      <section className="wa-ui-connection-summary" aria-labelledby={`${instance}-connection`}>
        <div className="wa-ui-connection-heading">
          <div className="wa-ui-connection-identity"><MessageCircle size={16} aria-hidden="true" /><span className="wa-ui-mono">{sessionId}</span>{session && <SessionBadge status={status} />}</div>
          <h2 id={`${instance}-connection`} className="wa-ui-connection-title">{session ? CONNECTION_COPY[status].title : 'Connection unavailable'}</h2>
          <p className="wa-ui-connection-copy">{session ? CONNECTION_COPY[status].detail : 'Refresh this session to check its connection and pairing options.'}</p>
        </div>
        {session && <div className="wa-ui-connection-actions">
          <div className="wa-ui-actions">
            <Button onClick={() => void lifecycle('start')} disabled={!canStart || !!operation}><Play size={16} aria-hidden="true" />{operation === 'start' ? 'Starting…' : 'Start session'}</Button>
            <Button variant="ghost" onClick={() => void lifecycle('stop')} disabled={!canStop || !!operation}><Square size={16} aria-hidden="true" />{operation === 'stop' ? 'Stopping…' : 'Stop session'}</Button>
          </div>
          <p className="wa-ui-note">Starting connects to WhatsApp. Stopping keeps your credentials.</p>
        </div>}
      </section>

      {(status === 'pairing' || status === 'starting') && <section className="wa-ui-linking-card" aria-labelledby={`${instance}-pairing`}>
        <header className="wa-ui-linking-header"><h3 id={`${instance}-pairing`} className="wa-ui-title">Choose how to link your phone</h3><p className="wa-ui-note">Use the camera in WhatsApp, or request a code instead.</p></header>
        <div className="wa-ui-pairing">
          <div className="wa-ui-qr-area">
            {validQr ? <><PairingQr pair={validQr} /><p className="wa-ui-note">Expires in <span className="wa-ui-mono">{formatDuration(validQr.expires_at_ms - now)}</span>.</p></> : <EmptyState compact title={status === 'starting' ? 'Waiting for the connection' : 'Waiting for a fresh QR code'} description="Pairing updates automatically. If a new QR code does not appear, stop and start the session." />}
            <ol className="wa-ui-pairing-steps"><li>Open WhatsApp on your phone.</li><li>Go to Linked devices and choose Link a device.</li><li>Scan this QR code to connect this session.</li></ol>
          </div>
          <div className="wa-ui-phone-pairing wa-ui-stack">
            <h4 className="wa-ui-title">Prefer a phone number?</h4>
            <p className="wa-ui-note">Request a code, then choose the option to link with a phone number in WhatsApp on your phone.</p>
            <SettingsList><SettingsField field="pairing_phone" label="Account phone number" description="Include the country code, without spaces." controlSize="full" layout="stacked" renderControl={(control) => <Input {...control} type="tel" inputMode="tel" autoComplete="off" value={phone} placeholder="+15551234567" disabled={!pairAllowed || operation === 'pair'} onChange={(next) => { pairScope.current.invalidate(); setPhone(next); setPhonePair(null); setActionError(null) }} />} /></SettingsList>
            <Button variant="ghost" onClick={() => void requestCode()} disabled={!pairAllowed || !!operation || !phone.trim()}>{operation === 'pair' ? 'Requesting code…' : 'Request pairing code'}</Button>
            {validPhonePair && <StatusPanel variant="info" headline={<span className="wa-ui-pair-code">{validPhonePair.code}</span>} detail={`Valid for ${formatDuration(validPhonePair.expires_at_ms - now)}. The code clears when it expires or the connection changes.`} />}
          </div>
        </div>
      </section>}

      <section className="wa-ui-activity-panel" aria-labelledby={`${instance}-activity`}>
        <header className="wa-ui-activity-heading"><div><h3 id={`${instance}-activity`} className="wa-ui-title">Recent activity</h3><p className="wa-ui-note">Connection changes, incoming messages, and delivery receipts.</p></div><span className="wa-ui-note">{feed.events.length} events</span></header>
        {feed.truncated && <StatusPanel variant="warn" headline="Some earlier activity is unavailable" detail="Activity is held in memory with a retention limit. This list shows the events still available after a retention gap or restart." />}
        <div className="wa-ui-activity-list">{feed.events.length === 0 ? <EmptyState compact icon={MessageCircle} title="No recent activity" description="Connection and message events appear here. The latest 100 events stay visible." /> : <List aria-label="Recent session activity">{[...feed.events].reverse().map((event) => <ListItem as="div" key={event.sequence} label={EVENT_LABELS[event.kind] ?? event.kind} description={eventDetail(event)} trailing={<time dateTime={new Date(event.timestamp_ms).toISOString()} title={new Date(event.timestamp_ms).toLocaleString('en-US')}>{formatRelative(event.timestamp_ms, now)}</time>} />)}</List>}</div>
      </section>
      <ConnectionHelp />
    </div>
    <LiveRegion announcement={announcement} />
  </div>
}
