import { Badge, Button, EmptyState, IconButton, Input, List, ListItem, LiveRegion, SettingsField, Skeleton, StatusPanel, type Host, type PageCommandsApi, type LiveAnnouncement } from '@iii-dev/console-ui'
import { errorMessage, formatDuration, formatRelative } from '@iii-dev/console-ui/format'
import { useEffect, useId, useRef, useState } from 'react'
import { ArrowLeft, ArrowRight, MessageCircle, Play, Smartphone, Square } from 'lucide-react'
import QRCode from 'qrcode'
import { eventDetail, pairingIsCurrent, pairingPhone, requestScope, STATUS_LABEL, type PairingCode, type SessionStatus } from './model'
import { useSession } from './use-session'
import { useSetupMotion } from './motion'

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
  stopped: { title: 'Connect on your terms.', detail: 'Start this session to connect an account. Saved credentials are reused when available.' },
  starting: { title: 'Opening your connection.', detail: 'Keep WhatsApp on your phone nearby. Pairing options appear below if this session needs to link an account.' },
  pairing: { title: 'Link your WhatsApp account.', detail: 'Scan the QR code or use your phone number to link this session. Choose the pairing method that works for you.' },
  connected: { title: 'Your account is connected.', detail: 'Open Messages to read available conversations, or Groups to explore the groups this account belongs to.' },
  disconnected: { title: 'Ready to reconnect.', detail: 'Start the session to reconnect. Your available message archive stays readable while the account is offline.' },
  stopping: { title: 'Closing the connection.', detail: 'The session is stopping. Its credentials remain saved for the next connection.' },
  logged_out: { title: 'Link your account again.', detail: 'This session is no longer linked to WhatsApp. Start it and pair your phone to reconnect.' },
  failed: { title: 'Let’s restore the connection.', detail: 'Check the error and recent activity, then start the session when you are ready to try again.' },
}

const CONNECTION_TIPS = [
  { title: 'Link from your phone.', text: 'In WhatsApp, open Linked devices from Settings on iPhone or the menu on Android. Choose Link a device, then scan the QR code shown here.' },
  { title: 'Keep your session.', text: 'Stopping a session keeps its saved credentials. Start the same session when you need to reconnect; creating another one is not necessary.' },
  { title: 'Give history time to arrive.', text: 'Messages shows what this session has captured and the history WhatsApp has shared. Request older history from a conversation; it may arrive later.' },
]

function ConnectionHelp() {
  const [index, setIndex] = useState(0)
  const tip = CONNECTION_TIPS[index]
  return <section className="wa-ui-connection-help wa-ui-stack-card" aria-label="Connection help" aria-roledescription="carousel">
    <div className="wa-ui-help-controls">
      <span className="wa-ui-note">A little help getting started</span>
      <div className="wa-ui-help-navigation">
        <IconButton label="Previous connection tip" onClick={() => setIndex((value) => (value + CONNECTION_TIPS.length - 1) % CONNECTION_TIPS.length)}><ArrowLeft size={16} /></IconButton>
        <span className="wa-ui-note" aria-label={`Tip ${index + 1} of ${CONNECTION_TIPS.length}`}>{index + 1} / {CONNECTION_TIPS.length}</span>
        <IconButton label="Next connection tip" onClick={() => setIndex((value) => (value + 1) % CONNECTION_TIPS.length)}><ArrowRight size={16} /></IconButton>
      </div>
    </div>
    <div className="wa-ui-help-slide" aria-live="polite" aria-atomic="true">
      <Smartphone size={16} aria-hidden="true" />
      <h3 className="wa-ui-help-title">{tip.title}</h3>
      <p className="wa-ui-connection-copy">{tip.text}</p>
    </div>
  </section>
}

export function SessionDetail({ host, sessionId, refreshList, commands }: {
  host: Host; sessionId: string; refreshList(): void; commands?: PageCommandsApi
}) {
  const root = useRef<HTMLDivElement>(null)
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
  useSetupMotion(root, `${sessionId}:${!!session}:${status}`)
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

  if (!session && loading) return <div ref={root} className="wa-ui-content wa-ui-connection"><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></div>
  return <div ref={root} className="wa-ui-content wa-ui-connection">
    {error && <StatusPanel variant="alert" headline="Could not refresh this session" detail={error} action={<Button variant="ghost" onClick={refresh}>Try again</Button>} />}
    {actionError && <StatusPanel variant="alert" headline="The operation did not complete" detail={actionError} action={<Button variant="ghost" onClick={() => setActionError(null)}>Dismiss</Button>} />}
    <div className="wa-ui-connection-grid">
      <section className="wa-ui-connection-summary" aria-labelledby={`${instance}-connection`}>
        <div className="wa-ui-connection-identity"><MessageCircle size={16} aria-hidden="true" /><span className="wa-ui-mono">{sessionId}</span>{session && <SessionBadge status={status} />}</div>
        <h2 id={`${instance}-connection`} className="wa-ui-connection-title wa-ui-reveal-text">{session ? CONNECTION_COPY[status].title : 'Your connection is unavailable.'}</h2>
        <p className="wa-ui-connection-copy">{session ? CONNECTION_COPY[status].detail : 'Refresh this session to check its connection and pairing options.'}</p>
        {session && <>
          <div className="wa-ui-actions">
            <Button onClick={() => void lifecycle('start')} disabled={!canStart || !!operation}><Play size={16} aria-hidden="true" />{operation === 'start' ? 'Starting…' : 'Start session'}</Button>
            <Button variant="ghost" onClick={() => void lifecycle('stop')} disabled={!canStop || !!operation}><Square size={16} aria-hidden="true" />{operation === 'stop' ? 'Stopping…' : 'Stop session'}</Button>
          </div>
          <p className="wa-ui-note">Starting connects to WhatsApp. Stopping keeps your credentials.</p>
        </>}
      </section>
      <ConnectionHelp />

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
            <SettingsField field="pairing_phone" label="Account phone number" description="Include the country code, without spaces." controlSize="full" layout="stacked" renderControl={(control) => <Input {...control} type="tel" inputMode="tel" autoComplete="off" value={phone} placeholder="+15551234567" disabled={!pairAllowed || operation === 'pair'} onChange={(next) => { pairScope.current.invalidate(); setPhone(next); setPhonePair(null); setActionError(null) }} />} />
            <Button variant="ghost" onClick={() => void requestCode()} disabled={!pairAllowed || !!operation || !phone.trim()}>{operation === 'pair' ? 'Requesting code…' : 'Request pairing code'}</Button>
            {validPhonePair && <StatusPanel variant="info" headline={<span className="wa-ui-pair-code">{validPhonePair.code}</span>} detail={`Valid for ${formatDuration(validPhonePair.expires_at_ms - now)}. The code clears when it expires or the connection changes.`} />}
          </div>
        </div>
      </section>}

      <section className="wa-ui-activity-panel" aria-labelledby={`${instance}-activity`}>
        <header className="wa-ui-activity-heading"><div><h3 id={`${instance}-activity`} className="wa-ui-title">Recent activity</h3><p className="wa-ui-note">Connection changes, incoming messages, and delivery receipts.</p></div><span className="wa-ui-note">{feed.events.length} events</span></header>
        {feed.truncated && <StatusPanel variant="warn" headline="Some earlier activity is unavailable" detail="Activity is held in memory with a retention limit. This list shows the events still available after a retention gap or restart." />}
        <div className="wa-ui-activity-list">{feed.events.length === 0 ? <EmptyState compact icon={MessageCircle} title="A quiet start" description="Activity will appear here as the session connects and receives messages. The latest 100 events stay visible." /> : <List aria-label="Recent session activity">{[...feed.events].reverse().map((event) => <ListItem as="div" key={event.sequence} label={EVENT_LABELS[event.kind] ?? event.kind} description={eventDetail(event)} trailing={<time dateTime={new Date(event.timestamp_ms).toISOString()} title={new Date(event.timestamp_ms).toLocaleString('en-US')}>{formatRelative(event.timestamp_ms, now)}</time>} />)}</List>}</div>
      </section>
    </div>
    <LiveRegion announcement={announcement} />
  </div>
}
