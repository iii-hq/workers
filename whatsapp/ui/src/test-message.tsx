import { useDraftRegistration, useDraftScope, useDraftState, usePageState } from './drafts'
import { Button, Input, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, ChevronDown, Copy, RefreshCw, Send } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { cloudRecipient, cloudTextError } from './cloud-model'
import { identityLabel, mergeMessages, type ChatTarget, type MessagePage, type MessageView } from './inbox-model'
import { normalizeRecipient, requestScope, validateMessage, type SentMessage } from './model'

const INITIAL_TEXT = 'Hello from iii. This is an integration test.'
type Attempt = 'none' | 'not_sent' | 'not_confirmed' | 'unknown'
interface TestRun extends SentMessage { sentText: string; startedAt: number; previousIncoming: Set<string> }
interface TestRecord {
  run: TestRun | null; messages: MessageView[]; pollError: string | null; lastRecipient: string; error: string | null; attempt: Attempt; attemptTarget: string | null
  recipientError: string | null; messageError: string | null; recipientTouched: boolean; messageTouched: boolean
}

export function testEvidence(run: TestRun, messages: MessageView[]) {
  const sameChat = messages.filter((message) => message.chat_id === run.chat_id)
  const outgoing = sameChat.find((message) => message.from_me && message.message_id === run.message_id)
  const incoming = sameChat.find((message) => !message.from_me && message.source === 'live'
    && !run.previousIncoming.has(message.message_id) && message.timestamp_ms >= Math.floor(run.startedAt / 1000) * 1000)
  const status = outgoing?.status
  return { incoming, failed: status === 'failed' || status === 'error', delivered: ['delivered', 'delivery', 'read', 'played'].includes(status ?? ''), read: status === 'read' || status === 'played' }
}

/** A test is explicit; subsequent polling only reads this conversation's archive. */
export function TestMessage({ host, transport, sessionId, ready, onDirty, onOpenConversation }: {
  host: Host; transport: 'native' | 'cloud'; sessionId?: string; ready: boolean
  onDirty?(value: boolean): void; onOpenConversation?(chat: ChatTarget): void
}) {
  const draftKey = `${useDraftScope() || `${transport}:${sessionId ?? 'cloud'}`}:test`
  const cloud = transport === 'cloud'
  const recipientValidation = (value: string) => {
    try { cloud ? cloudRecipient(value) : normalizeRecipient(value); return null }
    catch (cause) { return errorMessage(cause) }
  }
  const messageValidation = cloud ? cloudTextError : validateMessage
  const [record, setRecord] = usePageState<TestRecord>(draftKey, { run: null, messages: [], pollError: null, lastRecipient: '', error: null, attempt: 'none', attemptTarget: null, recipientError: null, messageError: null, recipientTouched: false, messageTouched: false })
  const [draft, setDraft, clearDraft] = useDraftState(draftKey, { recipient: '', recipientEdited: false, text: INITIAL_TEXT }, (value) => value.recipientEdited && !!value.recipient.trim() || value.text !== INITIAL_TEXT)
  const { run, messages, pollError, error, attempt, attemptTarget, recipientError, messageError } = record
  const { text } = draft
  const recipient = draft.recipientEdited ? draft.recipient : record.lastRecipient
  const dirty = text !== INITIAL_TEXT || draft.recipientEdited && !!recipient.trim()
  const setRecipient = (value: string) => {
    const blank = !value.trim()
    const changed = !blank && value.trim() !== record.lastRecipient.trim()
    setRecord((previous) => ({ ...previous, lastRecipient: blank ? '' : previous.lastRecipient, recipientError: previous.recipientTouched ? recipientValidation(value) : previous.recipientError }))
    setDraft((previous) => ({ ...previous, recipient: changed ? value : '', recipientEdited: changed }))
  }
  const setText = (value: string) => {
    setDraft((previous) => ({ ...previous, text: value }))
    setRecord((previous) => ({ ...previous, messageError: previous.messageTouched ? messageValidation(value) : previous.messageError }))
  }
  const [sending, setSending] = useState(false)
  const [revision, setRevision] = useState(0)
  const [checkingResults, setCheckingResults] = useState(false)
  const [completed, setCompleted] = useState(0)
  const results = useRef<HTMLDivElement>(null)
  const recipientInput = useRef<HTMLInputElement>(null)
  const messageInput = useRef<HTMLTextAreaElement>(null)
  const busy = useRef(false)
  const scope = useRef(requestScope())
  const instance = useId()
  const prefix = cloud ? 'whatsapp::cloud::messages' : 'whatsapp::messages'
  const context = cloud ? {} : { session_id: sessionId }
  const copy = useCopyFlash(run?.message_id ?? '')
  useDraftRegistration(draftKey, { dirty, busy: sending, clear: clearDraft })
  const evidence = run ? testEvidence(run, messages) : null
  const uncertain = attempt === 'unknown' || attempt === 'not_confirmed'
  const canSend = ready && !sending && !!recipient.trim() && !!text.trim()
  const refreshResults = () => { setCheckingResults(true); setRevision((value) => value + 1) }

  useLayoutEffect(() => {
    if (!completed || !results.current) return
    const summary = results.current
    const bounds = summary.getBoundingClientRect()
    if (!bounds.height) return
    let top = 0, bottom = window.innerHeight
    for (let parent = summary.parentElement; parent; parent = parent.parentElement) {
      if (!/(auto|scroll|hidden|clip)/.test(window.getComputedStyle(parent).overflowY)) continue
      const viewport = parent.getBoundingClientRect()
      if (viewport.height) { top = Math.max(top, viewport.top); bottom = Math.min(bottom, viewport.bottom) }
    }
    if (bounds.top < top || bounds.bottom > bottom) {
      summary.focus({ preventScroll: true })
      summary.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    }
  }, [completed])

  useEffect(() => () => scope.current.invalidate(), [])
  useEffect(() => {
    onDirty?.(sending || dirty)
    return () => onDirty?.(false)
  }, [sending, dirty, onDirty])
  useEffect(() => {
    if (!run) return
    let active = true, polling = false
    const poll = async () => {
      if (polling || document.hidden) return
      polling = true
      try {
        const page = await host.iii.trigger<MessagePage>(`${prefix}::list`, { ...context, chat_id: run.chat_id, limit: 50 })
        if (active) setRecord((previous) => {
          const reply = testEvidence(run, previous.messages).incoming
          const proof = previous.messages.filter((message) => message.from_me && message.message_id === run.message_id)
          if (reply) proof.push(reply)
          return { ...previous, messages: mergeMessages(proof, page.messages, run.chat_id), pollError: null }
        })
      } catch (cause) { if (active) setRecord((previous) => ({ ...previous, pollError: errorMessage(cause) })) }
      finally { polling = false; if (active) setCheckingResults(false) }
    }
    void poll()
    const timer = window.setInterval(() => void poll(), 2500)
    return () => { active = false; window.clearInterval(timer) }
  }, [host, prefix, sessionId, run, revision])

  const send = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!ready || busy.current) return
    const invalidRecipient = recipientValidation(recipient)
    const invalidMessage = messageValidation(text)
    if (invalidRecipient || invalidMessage) {
      setRecord((previous) => ({ ...previous, recipientError: invalidRecipient, messageError: invalidMessage, recipientTouched: true, messageTouched: true, error: null, attempt: 'not_sent', attemptTarget: null, run: null, messages: [], pollError: null }))
      if (invalidRecipient) recipientInput.current?.focus()
      else messageInput.current?.focus()
      return
    }
    const chatId = cloud ? cloudRecipient(recipient) : normalizeRecipient(recipient)
    const sentText = text
    busy.current = true
    const current = scope.current.begin()
    setSending(true); setRecord((previous) => ({ ...previous, error: null, recipientError: null, messageError: null, recipientTouched: true, messageTouched: true, attempt: 'none', attemptTarget: chatId, run: null, messages: [], pollError: null })); setCheckingResults(false)
    let dispatched = false
    try {
      const baseline = await host.iii.trigger<MessagePage>(`${prefix}::list`, { ...context, chat_id: chatId, limit: 50 })
      if (!current()) return
      const startedAt = Date.now()
      dispatched = true
      const result = await host.iii.trigger<SentMessage>(`${prefix}::send_text`, { ...context, chat_id: chatId, text: sentText }, { timeoutMs: cloud ? 120000 : 610000 })
      if (!current()) return
      setRecord((previous) => ({ ...previous, run: { ...result, sentText, startedAt, previousIncoming: new Set(baseline.messages.filter((message) => !message.from_me).map((message) => message.message_id)) }, lastRecipient: recipient.trim() }))
      clearDraft()
    } catch (cause) {
      if (current()) {
        const message = errorMessage(cause)
        const unknown = /timed?\s*out|timeout|outcome.*unknown|may have completed/i.test(message)
        setRecord((previous) => ({ ...previous, error: `${message}${dispatched ? ' Check the conversation before sending again. Sending again may duplicate the previous message. This test is never retried automatically.' : ' No message was sent.'}`, attempt: dispatched ? unknown ? 'unknown' : 'not_confirmed' : 'not_sent' }))
      }
    } finally { if (current()) { busy.current = false; setSending(false); setCompleted((value) => value + 1) } }
  }

  return <section className="wa-ui-test" aria-label={cloud ? 'Cloud API integration test' : 'Linked-device integration test'}>
    <div className="wa-ui-test-heading"><div><h2 className="wa-ui-title">Send a test message</h2>{!cloud && <p className="wa-ui-note">Use a phone you control, then reply from that phone to verify receiving.</p>}</div></div>
    <div className="wa-ui-test-grid">
      <form className="wa-ui-test-form" onSubmit={(event) => void send(event)}>
        <div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-recipient`}>Recipient</label><Input className="wa-ui-mono" ref={recipientInput} id={`${instance}-recipient`} aria-invalid={recipientError ? true : undefined} aria-describedby={`${instance}-recipient-help${recipientError ? ` ${instance}-recipient-error` : ''}`} value={recipient} onChange={setRecipient} onBlur={() => setRecord((previous) => ({ ...previous, recipientTouched: true, recipientError: recipientValidation(recipient) }))} placeholder={cloud ? '+15551234567' : 'Phone number or group ID'} autoComplete="off" disabled={sending} /><p id={`${instance}-recipient-help`} className="wa-ui-note">{cloud ? 'International phone number or business user ID.' : 'International phone number, or a group ID copied from Groups.'}</p>{recipientError && <p id={`${instance}-recipient-error`} className="wa-ui-note wa-ui-field-error" role="alert">{recipientError}</p>}</div>
        <div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-message`}>Test message</label><textarea ref={messageInput} id={`${instance}-message`} className="wa-ui-cloud-textarea" value={text} onChange={(event) => setText(event.target.value)} onBlur={() => setRecord((previous) => ({ ...previous, messageTouched: true, messageError: messageValidation(text) }))} rows={2} disabled={sending} aria-invalid={messageError ? true : undefined} aria-describedby={`${instance}-shortcut${messageError ? ` ${instance}-message-error` : ''}`} onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing && event.keyCode !== 229 && !event.repeat && canSend && !uncertain) {
            event.preventDefault()
            event.currentTarget.form?.requestSubmit()
          }
        }} /><p id={`${instance}-shortcut`} className="wa-ui-note wa-ui-cloud-send-shortcut">{uncertain ? 'Check the conversation before choosing Send again.' : 'Ctrl/Cmd+Enter to send'}</p>{messageError && <p id={`${instance}-message-error`} className="wa-ui-note wa-ui-field-error" role="alert">{messageError}</p>}</div>
        <div className="wa-ui-test-send"><Button type="submit" variant={uncertain ? 'ghost' : undefined} disabled={!canSend}><Send size={16} aria-hidden="true" />{sending ? 'Sending…' : uncertain ? 'Send again' : 'Send test message'}</Button><p className="wa-ui-note">{!ready ? cloud ? 'Complete Cloud API setup to send.' : 'Start and connect the session to send.' : cloud ? 'Sends a real Cloud API message.' : 'Sends a real message through the selected session.'}</p></div>
      </form>
      <div className="wa-ui-test-result" aria-live="polite" aria-label="Test results" aria-busy={checkingResults}>
        <div ref={results} className="wa-ui-test-summary" role="region" aria-label="Test results" tabIndex={-1}>
        <div className="wa-ui-test-result-heading"><h3 className="wa-ui-title">{pollError ? 'Last known test results' : run ? 'Latest test results' : 'Test results'}</h3>{run && <Button variant="ghost" aria-label="Refresh test results" onClick={refreshResults} disabled={checkingResults}><RefreshCw size={16} aria-hidden="true" /></Button>}</div>
        {pollError && <StatusPanel className="wa-ui-feedback" role="alert" variant="warn" headline="Could not verify results" detail={`${pollError} Delivery and incoming evidence shown is last known until the next successful check.`} action={<Button type="button" variant="ghost" onClick={refreshResults} disabled={checkingResults}>{checkingResults ? 'Checking…' : 'Check again'}</Button>} />}
        <dl className="wa-ui-test-checks">
          <div data-confirmed={!!run || undefined}><dt>{run && <Check size={16} aria-hidden="true" />}Send request</dt><dd>{run ? 'Accepted' : sending ? 'Sending…' : attempt === 'unknown' ? 'Outcome unknown' : attempt === 'not_confirmed' ? 'Not confirmed' : attempt === 'not_sent' ? 'Not sent' : 'Not tested'}</dd></div>
          <div data-confirmed={evidence?.delivered || undefined}><dt>{evidence?.delivered && <Check size={16} aria-hidden="true" />}Delivery</dt><dd>{evidence?.failed ? 'Failed' : evidence?.read ? 'Read' : evidence?.delivered ? 'Delivered' : pollError || attempt !== 'none' ? 'Unverified' : run ? 'Awaiting receipt' : 'Not tested'}</dd></div>
          <div data-confirmed={!!evidence?.incoming || undefined}><dt>{evidence?.incoming && <Check size={16} aria-hidden="true" />}Incoming message</dt><dd>{evidence?.incoming ? 'Captured' : pollError || attempt !== 'none' ? 'Unverified' : run ? 'Reply from the recipient' : 'Not tested'}</dd></div>
        </dl>
        {error && <StatusPanel className="wa-ui-feedback" role="alert" variant="alert" headline="Test not completed" detail={<>{error}{uncertain && attemptTarget && <p className="wa-ui-note">Attempted recipient: {identityLabel(attemptTarget)}</p>}</>} action={uncertain && attemptTarget && onOpenConversation ? <Button type="button" disabled={sending} onClick={() => onOpenConversation({ chat_id: attemptTarget, title: identityLabel(attemptTarget), is_group: attemptTarget.endsWith('@g.us') || attemptTarget.startsWith('group:') })}>Check this conversation</Button> : undefined} />}
        </div>
        {run ? <div className="wa-ui-test-evidence">
          <label className="wa-ui-composer-label" htmlFor={`${instance}-message-id`}>Message ID</label>
          <div className="wa-ui-test-id"><Input id={`${instance}-message-id`} value={run.message_id} onChange={() => {}} readOnly /><Button variant="ghost" aria-label="Copy test message ID" onClick={copy.copy}><Copy size={16} aria-hidden="true" />{copy.state === 'copied' ? 'Copied' : 'Copy'}</Button></div>
          {copy.state === 'failed' && <p className="wa-ui-note" role="status">Could not copy the message ID. Select it above and copy it manually.</p>}
          <p className="wa-ui-note">Recipient: {identityLabel(run.chat_id)}</p>
          <p id={`${instance}-sent-at`} className="wa-ui-note">Sent at: <time dateTime={new Date(run.startedAt).toISOString()}>{new Intl.DateTimeFormat('en-US', { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', timeZoneName: 'short' }).format(run.startedAt)}</time></p>
          <label className="wa-ui-composer-label" htmlFor={`${instance}-sent-message`}>Sent message</label>
          <textarea id={`${instance}-sent-message`} className="wa-ui-cloud-textarea wa-ui-test-sent-text" value={run.sentText} readOnly rows={2} wrap="soft" aria-describedby={`${instance}-sent-at`} />
          {identityLabel(run.chat_id) !== run.chat_id && <details className="wa-ui-cloud-setup"><summary>Recipient ID<ChevronDown size={16} aria-hidden="true" /></summary><Input aria-label="Raw recipient ID" value={run.chat_id} onChange={() => {}} readOnly /></details>}
          {evidence?.incoming && <p className="wa-ui-test-incoming">{evidence.incoming.text || 'Incoming message captured.'}</p>}
          {onOpenConversation && <Button variant="ghost" onClick={() => onOpenConversation({ chat_id: run.chat_id, title: identityLabel(run.chat_id), is_group: run.chat_id.endsWith('@g.us') || run.chat_id.startsWith('group:') })}>Open test conversation</Button>}
        </div> : <p className="wa-ui-note">Results come from this test. Stored history does not count as a new incoming message.</p>}
        {run && !evidence?.delivered && <p className="wa-ui-note">Acceptance confirms the send request. Delivery requires a receipt{cloud ? ' from the webhook' : ' from WhatsApp'}.</p>}
      </div>
    </div>
  </section>
}
