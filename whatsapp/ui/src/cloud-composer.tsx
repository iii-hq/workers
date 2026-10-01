import { Button, CodeEditor, EmptyState, Input, Select, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronDown, Info, Send } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { cloudRecipient, cloudTextError, templateComponents, templateInputs, templateInputComponents, templateText, type CloudSent, type CloudTemplate, type TemplatePage } from './cloud-model'
import type { ChatTarget } from './inbox-model'
import { requestScope } from './model'

export function CloudComposer({ host, chat, ready, onDirty, onSent, latestIncomingAt }: {
  host: Host; chat: ChatTarget | null; ready: boolean; onDirty(value: boolean): void; onSent(result: CloudSent): void; latestIncomingAt?: number
}) {
  const [recipient, setRecipient] = useState('')
  const [mode, setMode] = useState<'text' | 'template'>('text')
  const [text, setText] = useState('')
  const [components, setComponents] = useState('')
  const [values, setValues] = useState<Record<string, string>>({})
  const [templates, setTemplates] = useState<CloudTemplate[]>([])
  const [templateId, setTemplateId] = useState('')
  const [templateCursor, setTemplateCursor] = useState<string | null>(null)
  const [templatesLoaded, setTemplatesLoaded] = useState(false)
  const [loadingTemplates, setLoadingTemplates] = useState(false)
  const [templateError, setTemplateError] = useState<string | null>(null)
  const [sending, setSending] = useState(false)
  const [sendError, setSendError] = useState<{ message: string; uncertain: boolean } | null>(null)
  const [sent, setSent] = useState<CloudSent | null>(null)
  const [showWindowHelp, setShowWindowHelp] = useState(false)
  const [now, setNow] = useState(Date.now)
  const busy = useRef(false)
  const templateBusy = useRef(false)
  const templateAttempted = useRef(false)
  const sendScope = useRef(requestScope())
  const templateScope = useRef(requestScope())
  const instance = useId()
  const selected = templates.find((item) => item.id === templateId)
  const inputs = selected ? templateInputs(selected) : null
  const dirty = sending || !!text || !!components || !!templateId || !!recipient
  const windowOpen = latestIncomingAt !== undefined && latestIncomingAt <= now && now - latestIncomingAt < 24 * 60 * 60 * 1000
  const canSend = ready && !sending && !loadingTemplates && (!!chat || !!recipient.trim()) && (mode === 'text' ? !!text.trim() : selected?.status === 'APPROVED')

  useEffect(() => { onDirty(dirty); return () => onDirty(false) }, [dirty, onDirty])
  useEffect(() => () => { sendScope.current.invalidate(); templateScope.current.invalidate() }, [])
  useEffect(() => {
    const current = Date.now()
    setNow(current)
    if (latestIncomingAt === undefined || latestIncomingAt > current) return
    const remaining = latestIncomingAt + 24 * 60 * 60 * 1000 - current
    if (remaining <= 0) return
    const timer = window.setTimeout(() => setNow(Date.now()), remaining + 1)
    return () => window.clearTimeout(timer)
  }, [latestIncomingAt])

  const edit = () => { setSendError(null); setSent(null) }
  const loadTemplates = async (more = false) => {
    if (!ready || templateBusy.current) return
    templateBusy.current = true
    templateAttempted.current = true
    const current = templateScope.current.begin()
    setLoadingTemplates(true); setTemplateError(null)
    try {
      const page = await host.iii.trigger<TemplatePage>('whatsapp::cloud::templates::list', { limit: 50, after: more ? templateCursor ?? undefined : undefined })
      if (!current()) return
      setTemplates((previous) => [...new Map([...(more ? previous : []), ...page.templates].map((item) => [item.id, item])).values()])
      setTemplateCursor(page.next_cursor); setTemplatesLoaded(true)
    } catch (cause) { if (current()) setTemplateError(errorMessage(cause)) }
    finally { if (current()) { templateBusy.current = false; setLoadingTemplates(false) } }
  }
  useEffect(() => {
    if (mode === 'template' && ready && !templateAttempted.current) void loadTemplates()
  }, [mode, ready])

  const send = async () => {
    if (!canSend || busy.current) return
    setSendError(null); setSent(null)
    let chatId: string
    let parameters: ReturnType<typeof templateComponents>
    try {
      chatId = cloudRecipient(chat?.chat_id ?? recipient)
      if (mode === 'text') { const error = cloudTextError(text); if (error) throw new Error(error) }
      else {
        if (!selected || selected.status !== 'APPROVED') throw new Error('Choose an approved template before sending.')
        parameters = components.trim() ? templateComponents(components) : templateInputComponents(selected, values)
        if (inputs?.advancedRequired && !parameters?.length) throw new Error('Add the required components in Advanced JSON before sending.')
      }
    } catch (cause) { setSendError({ message: errorMessage(cause), uncertain: false }); return }
    busy.current = true
    const current = sendScope.current.begin()
    setSending(true)
    try {
      const result = mode === 'text'
        ? await host.iii.trigger<CloudSent>('whatsapp::cloud::messages::send_text', { chat_id: chatId, text })
        : await host.iii.trigger<CloudSent>('whatsapp::cloud::messages::send_template', { chat_id: chatId, name: selected!.name, language: selected!.language, components: parameters })
      if (!current()) return
      setText(''); setComponents(''); setValues({}); setTemplateId(''); setRecipient(''); setSent(result); onSent(result)
    } catch (cause) { if (current()) setSendError({ message: errorMessage(cause), uncertain: true }) }
    finally { if (current()) { busy.current = false; setSending(false) } }
  }

  return <section className="wa-ui-cloud-composer" aria-label="Send a Cloud message">
    <div className="wa-ui-composer-heading">
      <Select aria-label="Cloud message type" value={mode} options={[{ value: 'text', label: 'Text message' }, { value: 'template', label: 'Approved template' }]} onChange={(value) => { setMode(value as 'text' | 'template'); edit() }} disabled={sending} />
      <Button variant="ghost" className="wa-ui-cloud-help" aria-label={windowOpen ? 'Reply window open' : 'Reply window unknown'} title={windowOpen ? 'Reply window open' : 'Reply window unknown'} aria-expanded={showWindowHelp} aria-controls={`${instance}-window-help`} onClick={() => setShowWindowHelp((value) => !value)}><span>{windowOpen ? 'Reply window open' : 'Reply window unknown'}</span><Info size={16} aria-hidden="true" /></Button>
    </div>
    <div className="wa-ui-cloud-composer-fields">
      <p id={`${instance}-window-help`} className="wa-ui-note" hidden={!showWindowHelp}>{windowOpen ? 'A captured incoming message is less than 24 hours old. Free-form replies are available within that window.' : 'This local archive cannot confirm an open 24-hour reply window. Use an approved template if you are unsure.'} Meta confirms whether each message can be sent.</p>
      {!chat && <div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-recipient`}>Recipient</label><Input id={`${instance}-recipient`} value={recipient} onChange={(value) => { setRecipient(value); edit() }} placeholder="International phone number or business user ID" autoComplete="off" disabled={sending} /></div>}
      {mode === 'text' ? <div className="wa-ui-cloud-field">
        <label className="wa-ui-composer-label" htmlFor={`${instance}-text`}>Message</label>
        <textarea id={`${instance}-text`} value={text} onChange={(event) => { setText(event.target.value); edit() }} disabled={sending} className="wa-ui-cloud-textarea" aria-label="Cloud message text" placeholder="Write a message…" rows={2} onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing && event.keyCode !== 229 && !event.repeat) {
            event.preventDefault()
            void send()
          }
        }} />
      </div> : <div className="wa-ui-stack">
        {loadingTemplates && <p className="wa-ui-note" role="status">Loading templates…</p>}
        {templateError && <StatusPanel variant="alert" headline="Could not load templates" detail={templateError} action={<Button variant="ghost" onClick={() => void loadTemplates()} disabled={!ready || sending || loadingTemplates}>Retry templates</Button>} />}
        {templatesLoaded && !templates.length ? <EmptyState compact title="No templates found" description="Create and approve a template in WhatsApp Manager, then refresh this list." /> : <Select aria-label="Cloud message template" value={templateId || undefined} onChange={(value) => { setTemplateId(value); setComponents(''); setValues({}); edit() }} placeholder="Choose an approved template" disabled={sending || loadingTemplates || !templates.length} options={templates.map((item) => ({ value: item.id, label: `${item.name} · ${item.language}`, description: `${item.status} · ${item.category}`, disabled: item.status !== 'APPROVED' }))} />}
        {templatesLoaded && <div className="wa-ui-cloud-template-actions"><Button variant="ghost" onClick={() => void loadTemplates()} disabled={!ready || sending || loadingTemplates}>Refresh templates</Button>{templateCursor && <Button variant="ghost" onClick={() => void loadTemplates(true)} disabled={!ready || sending || loadingTemplates}>Load more templates</Button>}</div>}
        {selected && <>
          {templateText(selected) && <p className="wa-ui-cloud-template-preview">{templateText(selected)}</p>}
          {!inputs?.advancedRequired && !!inputs?.fields.length && <div className="wa-ui-cloud-parameter-fields">{inputs.fields.map((field) => <div className="wa-ui-cloud-field" key={field.key}><label className="wa-ui-composer-label" htmlFor={`${instance}-${field.key}`}>{field.label}</label><Input id={`${instance}-${field.key}`} value={values[field.key] ?? ''} disabled={sending || !!components.trim()} onChange={(value) => { setValues((previous) => ({ ...previous, [field.key]: value })); edit() }} /></div>)}</div>}
          <details key={selected.id} className="wa-ui-cloud-setup" open={inputs?.advancedRequired || undefined}><summary>Advanced JSON<ChevronDown size={16} aria-hidden="true" /></summary><div className="wa-ui-stack"><p className="wa-ui-note">{inputs?.advancedRequired ? 'This template uses media, dynamic buttons, or other components that need explicit send parameters. Add the required component objects before sending.' : 'Optional: replace the field values with an explicit array of send components. Leave empty to use the fields above.'}</p><CodeEditor language="json" value={components} onChange={(value) => { setComponents(value); edit() }} aria-label="Cloud template parameters" className="wa-ui-cloud-json" disabled={sending} placeholder="[]" /></div></details>
        </>}
      </div>}
      {sendError && <StatusPanel variant="alert" headline={sendError.uncertain ? 'Send not confirmed' : 'Message not sent'} detail={`${sendError.message}${sendError.uncertain ? ' Check the conversation before retrying; this message will not be sent again automatically.' : ''}`} />}
      {sent && <StatusPanel className="wa-ui-cloud-result" variant="success" headline="Accepted by Meta" detail={`Recipient: ${sent.chat_id}. Acceptance does not confirm delivery; updates appear in the conversation.`} />}
    </div>
    <div className="wa-ui-cloud-send"><p className="wa-ui-note">{!ready ? 'Configure the Cloud API before sending.' : mode === 'text' ? <>{Array.from(text).length.toLocaleString('en-US')} / 4,096<span className="wa-ui-cloud-send-shortcut"> · Ctrl/Cmd+Enter to send</span></> : 'Approved template'}</p><Button onClick={() => void send()} disabled={!canSend}><Send size={16} aria-hidden="true" />{sending ? 'Sending…' : mode === 'template' ? 'Send template' : 'Send'}</Button></div>
  </section>
}
