import {
  Button,
  CodeEditor,
  Input,
  KeyCombo,
  type PageRenderProps,
  SegmentedControl,
  Select,
  ShadcnTextarea,
  TerminalCommandLine,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { type ReactNode, useEffect, useId, useRef, useState } from 'react'
import { type FunctionAccess, SOURCE_TEXT } from './access'
import type { Api } from './api'
import { asCliCommand } from './cli'
import { runConfirm } from './presentation'
import { ResultView } from './ResultView'
import { CopyIconButton } from './SchemaTable'
import {
  type FormField,
  fieldsFromSchema,
  fieldText,
  invalidText,
  missingRequired,
  optionalLabel,
  parsePayload,
  requiredText,
  setField,
} from './schema-form'
import type { Server } from './types'

type Payload = Record<string, unknown>

interface Outcome {
  result: unknown
  error: string | null
  ms: number
}

const pretty = (value: unknown) => JSON.stringify(value, null, 2)

const MODES = [
  { value: 'form' as const, label: 'Form' },
  { value: 'json' as const, label: 'JSON' },
]
const BOOLEAN = [
  { value: 'unset', label: 'Unset' },
  { value: 'true', label: 'True' },
  { value: 'false', label: 'False' },
]

/** One input of the generated form. A typed field keeps its own text, so `1.` or a new line survives
 * until it reads as a value; the payload gets what it reads as on every keystroke. */
function Field({
  field,
  payload,
  onPayload,
  bad,
  onBad,
  running,
}: {
  field: FormField
  payload: Payload
  onPayload: (next: Payload) => void
  bad: boolean
  onBad: (name: string, bad: boolean) => void
  running: boolean
}) {
  const id = useId()
  const [text, setText] = useState(() => fieldText(payload, field))
  const write = (raw: string | boolean | null) => onPayload(setField(payload, field, raw))
  const type = (raw: string) => {
    setText(raw)
    try {
      write(raw)
      onBad(field.name, false)
    } catch {
      // Only a `json` field throws: the panel holds Run until its text parses.
      onBad(field.name, true)
    }
  }

  let control: ReactNode
  if (field.kind === 'enum') {
    control = (
      <Select
        id={id}
        value={fieldText(payload, field) || undefined}
        options={(field.enumValues ?? []).map((value) => ({ value, label: value }))}
        onChange={write}
        allowEmpty
        emptyLabel="Unset"
        onClear={() => write('')}
        placeholder={field.hint}
        // The disabled fieldset alone does not hold it: Base UI's trigger opens on press without asking the fieldset.
        disabled={running}
      />
    )
  } else if (field.kind === 'boolean') {
    control = (
      <SegmentedControl
        variant="radio"
        aria-label={field.label}
        value={fieldText(payload, field) || 'unset'}
        options={BOOLEAN}
        onChange={(next) => write(next === 'unset' ? null : next === 'true')}
      />
    )
  } else if (field.kind === 'string-list') {
    control = <ShadcnTextarea id={id} value={text} rows={2} onChange={(event) => type(event.target.value)} />
  } else if (field.kind === 'json') {
    control = (
      <>
        <CodeEditor
          id={id}
          className="mcp-ui-field-json"
          language="json"
          value={text}
          onChange={type}
          readOnly={running}
          aria-label={field.label}
        />
        {bad ? <p className="mcp-ui-input-error">Not valid JSON</p> : null}
      </>
    )
  } else {
    control = (
      <Input
        id={id}
        value={text}
        onChange={type}
        placeholder={field.hint}
        aria-required={field.required || undefined}
      />
    )
  }

  return (
    <div className="mcp-ui-field">
      {/* A name with no title shows once, in mono. */}
      <label className="mcp-ui-field-label" htmlFor={field.kind === 'boolean' ? undefined : id}>
        {field.label === field.name ? null : <span>{field.label}</span>}
        <code>{field.name}</code>
      </label>
      {control}
    </div>
  )
}

/** The Test tab: the Input card (a form or the payload as JSON, then Run), the Result card, and the CLI line under
 * both. */
export function TestPanel({
  api,
  server,
  functionId,
  name,
  schema,
  access,
  commands,
  runLabel,
  input,
}: {
  api: Api
  server: Server
  functionId: string
  name: string
  schema: unknown
  access: FunctionAccess
  commands: PageRenderProps['commands']
  runLabel?: string
  input?: (payload: Payload, setPayload: (next: Payload) => void) => ReactNode
}) {
  const label = runLabel ?? 'Run tool'
  const [payload, setPayload] = useState<Payload>({})
  // The JSON view's text and why it is not a payload; null while the form shows.
  const [json, setJson] = useState<{ text: string; error: string | null } | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  // The `json` fields whose text does not parse.
  const [bad, setBad] = useState<string[]>([])
  const [running, setRunning] = useState(false)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const seq = useRef(0)
  const confirming = useRef(false)
  // What had the focus when Run was asked for: disabling it while the call runs drops the focus.
  const trigger = useRef<Element | null>(null)
  const { confirm, dialog } = useConfirm()

  // A reply that arrives after the panel went away is dropped.
  useEffect(
    () => () => {
      seq.current += 1
    },
    [],
  )

  const update = (next: Payload) => {
    setPayload(next)
    setNotice(null)
  }
  const markBad = (name: string, isBad: boolean) =>
    setBad((names) =>
      names.includes(name) === isBad ? names : isBad ? [...names, name] : names.filter((n) => n !== name),
    )
  // Text that does not parse, in the JSON view or a field, holds Run and keeps the view as it is.
  const stuck = Boolean(json?.error) || bad.length > 0
  const editJson = (text: string) => {
    const parsed = parsePayload(text)
    if ('payload' in parsed) update(parsed.payload)
    setJson({ text, error: 'error' in parsed ? parsed.error : null })
  }

  const run = async () => {
    if (running || confirming.current || stuck) return
    trigger.current = document.activeElement
    const missing = missingRequired(schema, payload)
    if (missing.length > 0) {
      setNotice(requiredText(missing))
      return
    }
    const ask = runConfirm(server.name, name, access.cls)
    if (ask) {
      // Mod+Enter fires inside the open dialog too; a second request waits for this one.
      confirming.current = true
      const go = await confirm({
        ...ask,
        confirmLabel: label,
        description: <code className="mcp-ui-confirm-payload">{pretty(payload)}</code>,
      })
      confirming.current = false
      if (!go) return
    }
    const mine = ++seq.current
    const started = performance.now()
    setOutcome(null)
    setRunning(true)
    const settle = (result: unknown, error: string | null) => {
      if (mine !== seq.current) return
      setOutcome({ result, error, ms: performance.now() - started })
      setRunning(false)
    }
    api.call(functionId, payload).then(
      (result) => settle(result, null),
      (err) => settle(null, errorMessage(err)),
    )
  }

  useEffect(() => {
    const el = trigger.current
    if (!running && el instanceof HTMLElement && el.isConnected && document.activeElement === document.body) el.focus()
  }, [running])

  // The command reads the latest `run`, so it is registered again only when its title or state changes.
  const latestRun = useRef(run)
  useEffect(() => {
    latestRun.current = run
  })
  useEffect(
    () =>
      commands?.register([
        {
          id: 'run-test',
          title: label,
          shortcut: 'Mod+Enter',
          firesWhileTyping: true,
          enabled: () => !running,
          run: () => void latestRun.current(),
        },
      ]),
    [commands, label, running],
  )

  const { required, optional } = fieldsFromSchema(schema)
  const field = (f: FormField) => (
    <Field
      key={f.name}
      field={f}
      payload={payload}
      onPayload={update}
      bad={bad.includes(f.name)}
      onBad={markBad}
      running={running}
    />
  )

  const command = asCliCommand(functionId, payload)

  return (
    <div className="mcp-ui-test">
      <div className="mcp-ui-test-halves">
        <section className="mcp-ui-card" aria-label="Input">
          <div className="mcp-ui-card-head">
            <h3 className="mcp-ui-card-title">Input</h3>
            {input ? null : (
              <SegmentedControl
                variant="radio"
                aria-label="Input mode"
                value={json ? 'json' : 'form'}
                options={MODES}
                onChange={(mode) => {
                  if (!stuck) setJson(mode === 'json' ? { text: pretty(payload), error: null } : null)
                }}
              />
            )}
          </div>
          {bad.length > 0 ? (
            <p className="mcp-ui-input-error" role="alert">
              {invalidText(bad)}
            </p>
          ) : null}
          {json ? (
            <>
              <CodeEditor
                className="mcp-ui-payload-json"
                language="json"
                value={json.text}
                onChange={editJson}
                readOnly={running}
                aria-label="Input as JSON"
              />
              {json.error ? <p className="mcp-ui-input-error">{json.error}</p> : null}
            </>
          ) : (
            // A disabled fieldset disables every control in it, so the result belongs to the payload on screen.
            <fieldset className="mcp-ui-fields" disabled={running}>
              {input ? (
                input(payload, update)
              ) : required.length + optional.length === 0 ? (
                <p className="mcp-ui-quiet-line">
                  {access.kind === 'prompt' ? 'This prompt takes no arguments.' : 'This tool takes no inputs.'}
                </p>
              ) : (
                <>
                  {required.map(field)}
                  {optional.length > 0 ? (
                    <details className="mcp-ui-optional" open={required.length === 0 || undefined}>
                      <summary>{optionalLabel(required.length, optional.length, access.kind)}</summary>
                      <div className="mcp-ui-fields">{optional.map(field)}</div>
                    </details>
                  ) : null}
                </>
              )}
            </fieldset>
          )}
          <div className="mcp-ui-run">
            <Button onClick={() => void run()} disabled={running || stuck}>
              {label}
            </Button>
            <KeyCombo binding="Mod+Enter" />
          </div>
          {notice ? (
            <p className="mcp-ui-input-error" role="alert">
              {notice}
            </p>
          ) : null}
          <p className="mcp-ui-quiet-line">{SOURCE_TEXT[access.source]}</p>
        </section>
        <section className="mcp-ui-card" aria-label="Result">
          <ResultView
            result={outcome?.result ?? null}
            error={outcome?.error ?? null}
            ms={outcome?.ms ?? null}
            running={running}
          />
        </section>
      </div>
      <div className="mcp-ui-terminal">
        <h3 className="mcp-ui-card-title">From a terminal</h3>
        <TerminalCommandLine
          className="mcp-ui-cli"
          command={command}
          chips={<CopyIconButton value={command} label="Copy command" />}
        />
      </div>
      {dialog}
    </div>
  )
}
