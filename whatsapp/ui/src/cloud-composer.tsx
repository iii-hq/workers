import { Button, CodeEditor, EmptyState, Input, Select, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage, formatBytes } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { ChevronDown, Copy, Info, Paperclip, Send, X } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState, type SetStateAction } from 'react'
import { cloudRecipient, cloudTextError, templateInputs, templateSendComponents, templateStarterComponents, templateText, type CloudSent, type CloudTemplate, type TemplatePage } from './cloud-model'
import type { ChatTarget } from './inbox-model'
import { useDraftRegistration, useDraftScope, useDraftState, usePageState } from './drafts'
import { requestScope, type ConfigStatus } from './model'
import { CLOUD_ATTACHMENT_ACCEPT, cloudAttachmentSpec, readCloudAttachment, type CloudAttachmentSpec, type CloudUploadedAttachment } from './cloud-attachments'

export type CloudMessageMode = 'text' | 'template' | 'attachment'
interface CloudDraft {
  recipient: string; mode: CloudMessageMode; text: string; components: string; values: Record<string, string>; templateId: string
  file: File | null; attachment: CloudAttachmentSpec | null; uploaded: CloudUploadedAttachment | null; caption: string
}
const initialDraft = (mode: CloudMessageMode): CloudDraft => ({ recipient: '', mode, text: '', components: '', values: {}, templateId: '', file: null, attachment: null, uploaded: null, caption: '' })
const hasDraft = (draft: CloudDraft) => !!(draft.recipient || draft.text || draft.components || draft.templateId || draft.file)


export function CloudComposer({ host, chat, ready, onDirty, onSent, onCheckConversation, latestIncomingAt, initialMode = 'text', initialSent }: {
  host: Host; chat: ChatTarget | null; ready: boolean; onDirty(value: boolean): void; onSent(result: CloudSent, mode: CloudMessageMode): void; onCheckConversation?(target: ChatTarget): void; latestIncomingAt?: number; initialMode?: CloudMessageMode; initialSent?: CloudSent
}) {
  const modeIdentity = useRef(initialMode).current
  const accountScope = useDraftScope() || 'cloud'
  const draftKey = `${accountScope}:conversation:${chat?.chat_id ?? 'new'}:${modeIdentity}`
  const [preferredTemplateId, setPreferredTemplateId] = usePageState(`${accountScope}:template-choice`, '')
  const [draft, setDraft] = useDraftState(draftKey, initialDraft(initialMode), hasDraft)
  const [sendError, setSendError] = usePageState<{ message: string; uncertain: boolean; target?: ChatTarget } | null>(`${draftKey}:send-error`, null)
  const { recipient, mode, text, components, values, templateId: draftTemplateId, file, attachment, uploaded, caption } = draft
  const templateId = draftTemplateId || preferredTemplateId
  const field = <K extends keyof CloudDraft>(name: K) => (value: SetStateAction<CloudDraft[K]>) => setDraft((previous) => ({ ...previous, [name]: typeof value === 'function' ? (value as (previous: CloudDraft[K]) => CloudDraft[K])(previous[name]) : value }))
  const setRecipient = field('recipient'), setMode = field('mode'), setText = field('text'), setComponents = field('components'), setValues = field('values')
  const setTemplateId = field('templateId'), setFile = field('file'), setAttachment = field('attachment'), setUploaded = field('uploaded'), setCaption = field('caption')
  const clearDraft = () => setDraft(initialDraft(mode))
  const [recipientTouched, setRecipientTouched] = usePageState(`${draftKey}:recipient-touched`, false)
  const [touchedParameters, setTouchedParameters] = useState<Record<string, boolean>>({})
  const [templates, setTemplates] = useState<CloudTemplate[]>([])
  const [templateCursor, setTemplateCursor] = useState<string | null>(null)
  const [templatesLoaded, setTemplatesLoaded] = useState(false)
  const [loadingTemplates, setLoadingTemplates] = useState(false)
  const [templateError, setTemplateError] = useState<string | null>(null)
  const [sending, setSending] = useState(false)
  const [uploading, setUploading] = useState(false)
  const [checkingFile, setCheckingFile] = useState(false)
  const [fileError, setFileError] = useState<string | null>(null)
  const [sent, setSent] = useState<CloudSent | null>(() => hasDraft(draft) || sendError ? null : initialSent ?? null)
  const copy = useCopyFlash(sent?.message_id ?? '')
  const [showWindowHelp, setShowWindowHelp] = useState(false)
  const [now, setNow] = useState(Date.now)
  const busy = useRef(false)
  const templateBusy = useRef(false)
  const templateAttempted = useRef(false)
  const sendScope = useRef(requestScope())
  const templateScope = useRef(requestScope())
  const fileScope = useRef(requestScope())
  const fileInput = useRef<HTMLInputElement>(null)
  const fields = useRef<HTMLDivElement>(null)
  const receipt = useRef<HTMLDivElement>(null)
  const failure = useRef<HTMLDivElement>(null)
  const instance = useId()
  let recipientValidation: string | null = null
  if (!chat) {
    try { cloudRecipient(recipient) }
    catch (cause) { recipientValidation = errorMessage(cause) }
  }
  const recipientError = recipientTouched ? recipientValidation : null
  const selected = templates.find((item) => item.id === templateId)
  const inputs = selected ? templateInputs(selected) : null
  const componentExample = selected ? templateStarterComponents(selected) : ''
  let templateValidation: string | null = null
  if (mode === 'template' && selected) {
    try { templateSendComponents(selected, values, components) }
    catch (cause) { templateValidation = errorMessage(cause) }
  }
  const dirty = sending || hasDraft(draft)
  useDraftRegistration(draftKey, { dirty: hasDraft(draft), busy: sending || checkingFile, clear: clearDraft })
  const groupId = chat?.chat_id.startsWith('group:') ? chat.chat_id.slice(6) : null
  const windowOpen = latestIncomingAt !== undefined && latestIncomingAt <= now && now - latestIncomingAt < 24 * 60 * 60 * 1000
  const canSend = ready && !sending && !loadingTemplates && !recipientValidation && (!!chat || !!recipient.trim()) && (mode === 'text' ? !!text.trim() : mode === 'attachment' ? !!file && !!attachment && !checkingFile && !fileError : selected?.status === 'APPROVED' && !templateValidation)

  // Show new request feedback, never move the editor for archive polling.
  useLayoutEffect(() => {
    const feedback = sent ? receipt.current : sendError ? failure.current : null
    if (!fields.current || !feedback) return
    const viewport = fields.current.getBoundingClientRect()
    const bounds = feedback.getBoundingClientRect()
    if (!viewport.height || !bounds.height) return
    const outside = bounds.top < viewport.top || bounds.bottom > viewport.bottom
    if (outside) {
      // Oversized feedback starts with its headline; bottom alignment would hide it.
      const offset = bounds.height > viewport.height || bounds.top < viewport.top
        ? bounds.top - viewport.top : bounds.bottom - viewport.bottom
      fields.current.scrollTop = Math.max(0, fields.current.scrollTop + offset)
      feedback.focus({ preventScroll: true })
    }
  }, [sent?.message_id, sendError])

  useEffect(() => { onDirty(dirty); return () => onDirty(false) }, [dirty, onDirty])
  useEffect(() => () => { sendScope.current.invalidate(); templateScope.current.invalidate(); fileScope.current.invalidate() }, [])
  useEffect(() => {
    const current = Date.now()
    setNow(current)
    if (latestIncomingAt === undefined || latestIncomingAt > current) return
    const remaining = latestIncomingAt + 24 * 60 * 60 * 1000 - current
    if (remaining <= 0) return
    const timer = window.setTimeout(() => setNow(Date.now()), remaining + 1)
    return () => window.clearTimeout(timer)
  }, [latestIncomingAt])

  const edit = () => { setSendError((previous) => previous?.uncertain ? previous : null); setSent(null) }
  const selectFile = async (next: File | null) => {
    const current = fileScope.current.begin()
    setFile(next); setAttachment(null); setUploaded(null); setCaption(''); setFileError(null); setCheckingFile(false); edit()
    if (!next) { if (fileInput.current) fileInput.current.value = ''; return }
    try { cloudAttachmentSpec(next) } catch (cause) { setFileError(errorMessage(cause)); return }
    setCheckingFile(true)
    try {
      const status = await host.iii.trigger<ConfigStatus>('whatsapp::config-status')
      if (!current()) return
      const limit = status.active.max_media_bytes
      if (typeof limit !== 'number') throw new Error('The active upload limit could not be verified. Check the worker configuration before uploading.')
      setAttachment(cloudAttachmentSpec(next, limit))
    } catch (cause) { if (current()) setFileError(errorMessage(cause)) }
    finally { if (current()) setCheckingFile(false) }
  }
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
    let parameters: ReturnType<typeof templateSendComponents>
    try {
      if (groupId !== null && !/^[A-Za-z0-9][A-Za-z0-9_+/=-]{0,255}$/.test(groupId)) throw new Error('This Cloud group ID is invalid. Refresh the conversation before sending.')
      chatId = groupId !== null ? `group:${groupId}` : cloudRecipient(chat?.chat_id ?? recipient)
      if (mode === 'text') { const error = cloudTextError(text); if (error) throw new Error(error) }
      else if (mode === 'attachment') {
        if (!file || !attachment) throw new Error('Choose a supported file before sending.')
        if (attachment.caption && (Array.from(caption).length > 1024 || caption.includes('\0'))) throw new Error('Captions can contain up to 1,024 characters without NUL characters.')
      }
      else {
        if (!selected || selected.status !== 'APPROVED') throw new Error('Choose an approved template before sending.')
        parameters = templateSendComponents(selected, values, components)
      }
    } catch (cause) { setSendError({ message: errorMessage(cause), uncertain: false }); return }
    busy.current = true
    const current = sendScope.current.begin()
    setSending(true)
    let uploadStage = false
    try {
      const target = groupId !== null ? { group_id: groupId } : { chat_id: chatId }
      const messageNamespace = groupId !== null ? 'whatsapp::cloud::groups::messages' : 'whatsapp::cloud::messages'
      let result: CloudSent
      if (mode === 'attachment') {
        let media = uploaded
        if (!media) {
          uploadStage = true; setUploading(true)
          const dataBase64 = await readCloudAttachment(file!)
          if (!current()) return
          media = await host.iii.trigger<CloudUploadedAttachment>('whatsapp::cloud::media::upload', { media_type: attachment!.mediaType, data_base64: dataBase64, mimetype: attachment!.mimetype, file_name: file!.name })
          if (!current()) return
          if (!/^[0-9]{1,128}$/.test(media.media_id)) throw new Error('The upload did not return a valid media ID. No message was sent.')
          setUploaded(media); setUploading(false); uploadStage = false
        }
        const fields = { ...target, media_id: media.media_id, ...(attachment!.mediaType === 'document' ? { file_name: file!.name } : {}), ...(attachment!.caption && caption ? { caption } : {}) }
        result = groupId !== null
          ? await host.iii.trigger<CloudSent>(`${messageNamespace}::send_media`, { ...fields, media_type: attachment!.mediaType })
          : await host.iii.trigger<CloudSent>(`${messageNamespace}::send_${attachment!.mediaType}`, fields)
      } else result = mode === 'text'
        ? await host.iii.trigger<CloudSent>(`${messageNamespace}::send_text`, { ...target, text })
        : await host.iii.trigger<CloudSent>(`${messageNamespace}::send_template`, { ...target, name: selected!.name, language: selected!.language, components: parameters })
      if (!current()) return
      if (mode === 'template' && selected) setPreferredTemplateId(selected.id)
      clearDraft(); setRecipientTouched(false); setTouchedParameters({}); if (fileInput.current) fileInput.current.value = ''; setSent(result); onSent(result, mode)
    } catch (cause) { if (current()) setSendError({ message: `${errorMessage(cause)}${uploadStage ? ' No message was sent. The upload may have completed; it will not be repeated automatically.' : ''}`, uncertain: !uploadStage, target: { chat_id: chatId, title: chat?.title || chatId, is_group: groupId !== null } }) }
    finally { if (current()) { busy.current = false; setSending(false); setUploading(false) } }
  }

  return <section className="wa-ui-cloud-composer" data-feedback={!!(sent || sendError) || undefined} aria-label="Send a Cloud message">
    <div className="wa-ui-composer-heading">
      <Select aria-label="Cloud message type" value={mode} options={[{ value: 'text', label: 'Text message' }, { value: 'attachment', label: 'Attachment' }, { value: 'template', label: 'Approved template' }]} onChange={(value) => { setMode(value as CloudMessageMode); edit() }} disabled={sending} />
      <Button variant="ghost" className="wa-ui-cloud-help" aria-label={windowOpen ? 'Reply window open' : 'Reply window unknown'} title={windowOpen ? 'Reply window open' : 'Reply window unknown'} aria-expanded={showWindowHelp} aria-controls={`${instance}-window-help`} onClick={() => setShowWindowHelp((value) => !value)}><span>{windowOpen ? 'Window open' : 'Window unknown'}</span><Info size={16} aria-hidden="true" /></Button>
    </div>
    <div ref={fields} className="wa-ui-cloud-composer-fields">
      {sendError && <div ref={failure} className="wa-ui-cloud-feedback" role="region" aria-label="Cloud send error" tabIndex={-1}><StatusPanel className="wa-ui-feedback" role="alert" variant="alert" headline={sendError.uncertain ? 'Send not confirmed' : 'Message not sent'} detail={<>{sendError.message}{sendError.uncertain && <><p>Check the conversation before sending again. Sending again may duplicate the previous message; it will not be repeated automatically.</p>{sendError.target && <p>Attempted recipient: <span className="wa-ui-mono">{sendError.target.chat_id}</span></p>}</>}</>} /></div>}
      {sent && <div ref={receipt} className="wa-ui-cloud-result wa-ui-stack" role="region" aria-label="Cloud send result" tabIndex={-1}><StatusPanel variant="success" role="status" headline="Accepted by Meta" detail={`Recipient: ${sent.chat_id}. Acceptance does not confirm delivery; updates appear in the conversation.`} /><div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-message-id`}>Message ID</label><div className="wa-ui-test-id"><Input id={`${instance}-message-id`} value={sent.message_id} onChange={() => {}} readOnly /><Button variant="ghost" aria-label="Copy message ID" onClick={copy.copy}><Copy size={16} aria-hidden="true" />{copy.state === 'copied' ? 'Copied' : 'Copy'}</Button></div>{copy.state === 'failed' && <p className="wa-ui-note" role="status">Could not copy the message ID. Select it above and copy it manually.</p>}</div></div>}
      <p id={`${instance}-window-help`} className="wa-ui-note" hidden={!showWindowHelp}>{windowOpen ? 'A captured incoming message is less than 24 hours old. Free-form replies are available within that window.' : 'This local archive cannot confirm an open 24-hour reply window. Use an approved template if you are unsure.'} Meta confirms whether each message can be sent.</p>
      {!chat && <div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-recipient`}>Recipient</label><Input className="wa-ui-mono" id={`${instance}-recipient`} aria-invalid={recipientError ? true : undefined} aria-describedby={`${instance}-recipient-help${recipientError ? ` ${instance}-recipient-error` : ''}`} value={recipient} onChange={(value) => { setRecipient(value); edit() }} onBlur={() => setRecipientTouched(true)} placeholder="International phone number or business user ID" autoComplete="off" disabled={sending} /><p id={`${instance}-recipient-help`} className="wa-ui-note">International phone number or business user ID.</p>{recipientError && <p id={`${instance}-recipient-error`} className="wa-ui-note wa-ui-field-error" role="alert">{recipientError}</p>}</div>}
      {mode === 'text' ? <div className="wa-ui-cloud-field">
        <label className="wa-ui-composer-label" htmlFor={`${instance}-text`}>Message</label>
        <textarea id={`${instance}-text`} value={text} onChange={(event) => { setText(event.target.value); edit() }} disabled={sending} className="wa-ui-cloud-textarea" aria-label="Cloud message text" placeholder="Write a message…" rows={2} onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing && event.keyCode !== 229 && !event.repeat) {
            event.preventDefault()
            if (!sendError?.uncertain) void send()
          }
        }} />
      </div> : mode === 'attachment' ? <div className="wa-ui-stack">
        <div className="wa-ui-cloud-field">
          <label className="wa-ui-composer-label" htmlFor={`${instance}-attachment`}>Attachment</label>
          <input ref={fileInput} id={`${instance}-attachment`} type="file" accept={CLOUD_ATTACHMENT_ACCEPT} disabled={sending} className="wa-ui-cloud-file-input" aria-describedby={`${instance}-file-help`} onChange={(event) => void selectFile(event.target.files?.[0] ?? null)} />
          <p id={`${instance}-file-help`} className="wa-ui-note">Documents, images, audio, video or WebP stickers. Files are uploaded only when you send.</p>
        </div>
        {file && <div className="wa-ui-cloud-attachment-summary"><Paperclip size={16} aria-hidden="true" /><div><span className="wa-ui-cloud-attachment-name">{file.name}</span><p className="wa-ui-note">{formatBytes(file.size)}{attachment ? ` · Limit ${formatBytes(attachment.maxBytes)}` : ''}{uploaded ? ' · Uploaded, ready to send' : ''}</p></div><Button variant="ghost" onClick={() => void selectFile(null)} disabled={sending} aria-label="Remove attachment"><X size={16} aria-hidden="true" /></Button></div>}
        {checkingFile && <p className="wa-ui-note" role="status">Checking the upload limit…</p>}
        {fileError && <StatusPanel role="alert" variant="alert" headline="Attachment unavailable" detail={fileError} />}
        {attachment?.caption && <div className="wa-ui-cloud-field"><label className="wa-ui-composer-label" htmlFor={`${instance}-caption`}>Caption (optional)</label><textarea id={`${instance}-caption`} value={caption} onChange={(event) => { setCaption(event.target.value); edit() }} disabled={sending} className="wa-ui-cloud-textarea" rows={2} placeholder="Add a caption…" /></div>}
        {attachment && !attachment.caption && <p className="wa-ui-note">Audio and sticker messages do not support captions.</p>}
      </div> : <div className="wa-ui-stack">
        {loadingTemplates && <p className="wa-ui-note" role="status">Loading templates…</p>}
        {templateError && <StatusPanel className="wa-ui-feedback" role="alert" variant="alert" headline="Could not load templates" detail={templateError} action={<Button variant="ghost" onClick={() => void loadTemplates()} disabled={!ready || sending || loadingTemplates}>Retry templates</Button>} />}
        {templatesLoaded && !templates.length ? <EmptyState compact title="No templates found" description="Create and approve a template in WhatsApp Manager, then refresh this list." /> : <Select aria-label="Cloud message template" value={templateId || undefined} onChange={(value) => { setTemplateId(value); setComponents(''); setValues({}); setTouchedParameters({}); edit() }} placeholder="Choose an approved template" disabled={sending || loadingTemplates || !templates.length} options={templates.map((item) => ({ value: item.id, label: `${item.name} · ${item.language}`, description: `${item.status} · ${item.category}`, disabled: item.status !== 'APPROVED' }))} />}
        {templatesLoaded && <div className="wa-ui-cloud-template-actions"><Button variant="ghost" onClick={() => void loadTemplates()} disabled={!ready || sending || loadingTemplates}>Refresh templates</Button>{templateCursor && <Button variant="ghost" onClick={() => void loadTemplates(true)} disabled={!ready || sending || loadingTemplates}>Load more templates</Button>}</div>}
        {selected && <>
          {templateText(selected) && <div className="wa-ui-cloud-field"><span className="wa-ui-composer-label">{components.trim() ? 'Approved template text' : 'Template preview'}</span><p className="wa-ui-cloud-template-preview">{templateText(selected, components.trim() ? undefined : values)}</p></div>}
          {!inputs?.advancedRequired && !!inputs?.fields.length && <div className="wa-ui-cloud-parameter-fields">{inputs.fields.map((field) => {
            const usingJson = !!components.trim()
            const invalid = !usingJson && touchedParameters[field.key] && !(values[field.key] ?? '').trim()
            return <div className="wa-ui-cloud-field" key={field.key}><label className="wa-ui-composer-label" htmlFor={`${instance}-${field.key}`}>{field.label}</label><Input id={`${instance}-${field.key}`} value={values[field.key] ?? ''} required={!usingJson} aria-describedby={`${instance}-${field.key}-help`} aria-invalid={invalid || undefined} disabled={sending || usingJson} onBlur={() => setTouchedParameters((previous) => ({ ...previous, [field.key]: true }))} onChange={(value) => { setValues((previous) => ({ ...previous, [field.key]: value })); edit() }} /><p id={`${instance}-${field.key}-help`} className="wa-ui-note">{usingJson ? 'Using Advanced JSON instead.' : invalid ? `Enter a value for {{${field.parameter}}}.` : `Required. Replaces {{${field.parameter}}} in the ${field.component}.`}</p></div>
          })}</div>}
          <details key={selected.id} className="wa-ui-cloud-setup" open={inputs?.advancedRequired || undefined}><summary>Advanced JSON<ChevronDown size={16} aria-hidden="true" /></summary><div className="wa-ui-stack"><p id={`${instance}-template-json-help`} className="wa-ui-note">{inputs?.advancedRequired ? 'This template uses media, dynamic buttons, or other components that need explicit send parameters. Add the required component objects before sending.' : 'Optional: replace the field values with an explicit array of send components. Leave empty to use the fields above.'}</p><p className="wa-ui-note"><a href="https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/overview" target="_blank" rel="noopener noreferrer">Meta template payload reference</a></p>{componentExample && <div className="wa-ui-cloud-field"><span className="wa-ui-composer-label">Example for {selected.name}</span><pre className="wa-ui-cloud-component-example" aria-label="Selected template component example"><code>{componentExample}</code></pre><p className="wa-ui-note">Replace the empty values with your own parameters. This example is never inserted or sent automatically; some advanced templates need additional components from the Meta reference.</p></div>}<div role="group" aria-label="Advanced template components" aria-describedby={`${instance}-template-json-help${components.trim() && templateValidation ? ` ${instance}-template-json-validation` : ''}`}><CodeEditor language="json" value={components} onChange={(value) => { setComponents(value); edit() }} aria-label="Cloud template parameters" className="wa-ui-cloud-json" disabled={sending} placeholder="[]" /></div>{components.trim() && templateValidation && <p id={`${instance}-template-json-validation`} className="wa-ui-note wa-ui-field-error" role="status">{templateValidation}</p>}</div></details>
        </>}
      </div>}
    </div>
    <div className="wa-ui-cloud-send" data-uncertain={sendError?.uncertain || undefined}>{(!chat && recipient.trim() || mode === 'template' && selected) && <div className="wa-ui-cloud-send-context" role="group" aria-label="Next message details">{!chat && recipient.trim() && <p className="wa-ui-note">Next recipient: <span className="wa-ui-mono">{recipient.trim()}</span></p>}{mode === 'template' && selected && <p className="wa-ui-note">Template: <span className="wa-ui-mono">{selected.name} · {selected.language}</span></p>}</div>}<p className="wa-ui-note">{!ready ? 'Configure the Cloud API before sending.' : recipientValidation ? 'Enter a valid recipient to send.' : mode === 'text' ? <>{Array.from(text).length.toLocaleString('en-US')} / 4,096<span className="wa-ui-cloud-send-shortcut"> · {sendError?.uncertain ? 'Check the previous attempt before sending again' : 'Ctrl/Cmd+Enter to send'}</span></> : mode === 'attachment' ? attachment?.caption ? `${Array.from(caption).length.toLocaleString('en-US')} / 1,024 caption characters` : 'Cloud API attachment' : !selected ? 'Choose an approved template to send.' : templateValidation ? components.trim() ? 'Fix the template components to send.' : inputs?.advancedRequired ? 'Add the required Advanced JSON components to send.' : 'Complete the required template fields to send.' : 'Ready to send the approved template.'}</p>{sendError?.uncertain && sendError.target && onCheckConversation && <Button disabled={sending} onClick={() => onCheckConversation(sendError.target!)}>Check this conversation</Button>}<Button variant={sendError?.uncertain ? 'ghost' : undefined} onClick={() => void send()} disabled={!canSend}><Send size={16} aria-hidden="true" />{uploading ? 'Uploading…' : sending ? 'Sending…' : sendError?.uncertain ? 'Send again' : mode === 'template' ? 'Send template' : mode === 'attachment' ? 'Send attachment' : 'Send'}</Button></div>
  </section>
}
