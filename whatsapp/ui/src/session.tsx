import { Badge, Button, EmptyState, Input, List, ListItem, LiveRegion, MetaRow, SettingsField, Skeleton, StatusPanel, type Host, type PageCommandsApi, type LiveAnnouncement } from '@iii-dev/console-ui'
import { errorMessage, formatDuration, formatRelative } from '@iii-dev/console-ui/format'
import { useEffect, useId, useRef, useState } from 'react'
import { MessageCircle, Play, Square } from 'lucide-react'
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
  if (error) return <StatusPanel variant="alert" headline="Não foi possível desenhar o QR" detail={error} />
  return image?.code === pair.code
    ? <img className="wa-ui-qr" src={image.url} width={288} height={288} alt="QR de pareamento da sessão WhatsApp selecionada" />
    : <Skeleton className="wa-ui-qr" aria-label="Preparando QR de pareamento" />
}

const EVENT_LABELS: Record<string, string> = {
  'message.received': 'Mensagem recebida', 'message.receipt': 'Confirmação de mensagem',
  'session.created': 'Sessão criada', 'session.starting': 'Sessão iniciando',
  'session.connected': 'Sessão conectada', 'session.disconnected': 'Sessão desconectada',
  'session.stopped': 'Sessão parada', 'session.stopping': 'Sessão parando',
  'session.qr': 'QR disponível', 'session.qr_expired': 'QR expirado', 'session.pair_code': 'Código disponível',
  'session.pair_code_invalidated': 'Código invalidado', 'session.paired': 'Conta pareada',
  'session.pairing_failed': 'Pareamento falhou', 'session.logged_out': 'Conta desvinculada',
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
      setAnnouncement({ seq: Date.now(), text: action === 'start' ? 'Início da sessão solicitado.' : 'Sessão parada; credenciais preservadas.', urgency: 'polite' })
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
    { id: 'start', title: 'Iniciar sessão', shortcut: 'S', enabled: () => canStart && !busy.current, run: () => void lifecycle('start') },
    { id: 'stop', title: 'Parar sessão', shortcut: 'X', enabled: () => canStop && !busy.current, run: () => void lifecycle('stop') },
    { id: 'pair-code', title: 'Solicitar código de pareamento', enabled: () => pairAllowed && !!phone.trim() && !busy.current, run: () => void requestCode() },
  ]), [commands, canStart, canStop, pairAllowed, phone])

  if (!session && loading) return <div className="wa-ui-content"><Skeleton className="wa-ui-loading" /><Skeleton className="wa-ui-loading" /></div>
  return <div className="wa-ui-content">
    <div className="wa-ui-session-heading"><h2 className="wa-ui-session-id">{sessionId}</h2>{session && <SessionBadge status={status} />}</div>
    {error && <StatusPanel variant="alert" headline="Não foi possível atualizar esta sessão" detail={error} action={<Button variant="ghost" onClick={refresh}>Tentar novamente</Button>} />}
    {session && <>
      <div className="wa-ui-actions">
        <Button onClick={() => void lifecycle('start')} disabled={!canStart || !!operation}><Play size={16} aria-hidden="true" />{operation === 'start' ? 'Iniciando…' : 'Iniciar sessão'}</Button>
        <Button variant="ghost" onClick={() => void lifecycle('stop')} disabled={!canStop || !!operation}><Square size={16} aria-hidden="true" />{operation === 'stop' ? 'Parando…' : 'Parar sessão'}</Button>
      </div>
      <p className="wa-ui-note">Iniciar conecta esta sessão ao WhatsApp. Parar mantém as credenciais para a próxima conexão.</p>
    </>}
    {actionError && <StatusPanel variant="alert" headline="A operação não foi concluída" detail={actionError} action={<Button variant="ghost" onClick={() => setActionError(null)}>Dispensar</Button>} />}

    {(status === 'pairing' || status === 'starting') && <section className="wa-ui-section" aria-labelledby={`${instance}-pairing`}>
      <h3 id={`${instance}-pairing`}>Parear a conta</h3>
      <p className="wa-ui-note">No WhatsApp do celular, abra Aparelhos conectados e escolha Conectar aparelho. O QR é desenhado aqui; nenhum serviço externo recebe o código.</p>
      <div className="wa-ui-pairing">
        <div className="wa-ui-qr-area">{validQr ? <><PairingQr pair={validQr} /><p className="wa-ui-note">QR válido por <span className="wa-ui-mono">{formatDuration(validQr.expires_at_ms - now)}</span>.</p></> : <EmptyState compact title={status === 'starting' ? 'Aguardando conexão' : 'Aguardando um QR válido'} description="A sessão consulta o pareamento automaticamente. Se o QR não voltar, pare e inicie a sessão." />}</div>
        <div className="wa-ui-stack">
          <h4>Usar o número de telefone</h4>
          <p className="wa-ui-note">Alternativa ao QR: solicite um código e informe-o na opção de conectar com número de telefone no celular.</p>
          <SettingsField field="pairing_phone" label="Telefone da conta" controlSize="full" layout="stacked" renderControl={(control) => <Input {...control} type="tel" inputMode="tel" autoComplete="off" value={phone} placeholder="+5511999999999" disabled={!pairAllowed || operation === 'pair'} onChange={(next) => { pairScope.current.invalidate(); setPhone(next); setPhonePair(null); setActionError(null) }} />} />
          <Button variant="ghost" onClick={() => void requestCode()} disabled={!pairAllowed || !!operation || !phone.trim()}>{operation === 'pair' ? 'Solicitando código…' : 'Solicitar código'}</Button>
          {validPhonePair && <StatusPanel variant="info" headline={<span className="wa-ui-pair-code">{validPhonePair.code}</span>} detail={`Código válido por ${formatDuration(validPhonePair.expires_at_ms - now)}. Ele será removido ao expirar ou mudar a conexão.`} />}
        </div>
      </div>
    </section>}

    <section className="wa-ui-section" aria-labelledby={`${instance}-activity`}>
      <h3 id={`${instance}-activity`}>Atividade da sessão</h3>
      <MetaRow items={[{ label: 'Eventos visíveis', value: feed.events.length }, { label: 'Cursor', value: feed.cursor }]} />
      {feed.truncated && <StatusPanel variant="warn" headline="Parte do histórico não está mais disponível" detail="O histórico é limitado e fica em memória. Houve uma lacuna de retenção ou reinício; esta lista mostra os eventos ainda disponíveis." />}
      {feed.events.length === 0 ? <EmptyState compact icon={MessageCircle} title="Nenhuma atividade por enquanto" description="Mensagens recebidas, confirmações e mudanças da conexão aparecerão aqui. Os últimos 100 eventos ficam visíveis." /> : <List aria-label="Atividade recente">{[...feed.events].reverse().map((event) => <ListItem as="div" key={event.sequence} label={EVENT_LABELS[event.kind] ?? event.kind} description={eventDetail(event)} trailing={<time dateTime={new Date(event.timestamp_ms).toISOString()} title={new Date(event.timestamp_ms).toLocaleString('pt-BR')}>{formatRelative(event.timestamp_ms, now)}</time>} />)}</List>}
    </section>
    <LiveRegion announcement={announcement} />
  </div>
}
