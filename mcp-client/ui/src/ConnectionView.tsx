import {
  Button,
  CodeEditor,
  type Host,
  IconButton,
  Input,
  MetaRow,
  type SecretKeyFieldProps,
  SegmentedControl,
  SettingsField,
  type SettingsFieldControlProps,
  SettingsList,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { CircleCheck, Plus, X } from 'lucide-react'
import { type ComponentType, type ReactNode, useEffect, useId, useMemo, useRef, useState } from 'react'
import type { Api } from './api'
import {
  type Draft,
  draftDirty,
  entryFromDraft,
  fieldErrors,
  hideLiterals,
  readEntryEdit,
  secretVaultName,
  unplacedErrors,
} from './config-model'
import { effectLine } from './effect-line'
import { KeyValueRows, SecretValue } from './KeyValueRows'
import { advertisesText, stageTrack } from './presentation'
import { effectSentences, removeLines } from './save'
import type { Issue, Server } from './types'

const MODES = [
  { value: 'form' as const, label: 'Form', icon: false as const },
  { value: 'json' as const, label: 'JSON', icon: false as const },
]
const TRANSPORTS = [
  { value: 'http' as const, label: 'HTTP', icon: false as const },
  { value: 'stdio' as const, label: 'stdio', icon: false as const },
]
const AUTH_MODES = [
  { value: 'none' as const, label: 'None', icon: false as const },
  { value: 'bearer' as const, label: 'API key', icon: false as const },
  { value: 'client_credentials' as const, label: 'Client credentials', icon: false as const },
  { value: 'oauth' as const, label: 'Sign in', icon: false as const },
]

/** The JSON view's text of a draft: a value in a secret position hidden. */
const jsonText = (draft: Draft) => JSON.stringify(hideLiterals(entryFromDraft(draft)), null, 2)

/** Sentences, one per line, in a panel's detail or a confirm's description. */
export const Lines = ({ lines }: { lines: string[] }) =>
  lines.map((line) => (
    <span key={line} className="mcp-ui-line">
      {effectLine(line)}
    </span>
  ))

/** The save bar: Discard, and Save as the view's one default button. `blocked` says why Save cannot run. */
export function SaveBar({
  onSave,
  onDiscard,
  saving,
  blocked,
}: {
  onSave: () => void
  onDiscard: () => void
  saving: boolean
  blocked: string | null
}) {
  return (
    <div className="mcp-ui-save-bar">
      <span>Unsaved changes</span>
      {blocked ? <span>{blocked}</span> : null}
      <Button variant="ghost" size="sm" disabled={saving} onClick={onDiscard}>
        Discard
      </Button>
      <Button size="sm" disabled={saving || blocked !== null} onClick={onSave}>
        {saving ? 'Saving…' : 'Save'}
      </Button>
    </div>
  )
}

/** What an entry changed outside the page would do once approved, and Approve. `quiet` while the save bar
 * shows: Save is then the view's default button. */
export function ReviewPanel({
  api,
  server,
  entry,
  busy,
  onApprove,
  quiet = false,
}: {
  api: Api
  server: Server
  entry: unknown
  busy: boolean
  onApprove: () => void
  quiet?: boolean
}) {
  const [lines, setLines] = useState<string[]>([])
  const seq = useRef(0)
  // Every refresh hands over a new entry object: only a changed entry is validated again, and the lines shown
  // stay until the new ones arrive (Approve asks again with fresh ones).
  const text = useMemo(() => JSON.stringify(entry), [entry])
  useEffect(() => {
    const mine = ++seq.current
    api.validate({ [server.name]: JSON.parse(text ?? 'null') }).then(
      ({ servers }) => {
        const effects = servers[server.name]?.effects
        if (mine === seq.current && effects) setLines(effectSentences(effects))
      },
      () => {},
    )
  }, [api, server.name, text])
  return (
    <StatusPanel
      variant="warn"
      headline="Review needed"
      detail={<Lines lines={['This entry was changed outside this page.', ...lines]} />}
      action={
        <Button size="sm" variant={quiet ? 'secondary' : 'primary'} disabled={busy} onClick={onApprove}>
          Approve
        </Button>
      }
    />
  )
}

/** Column 3 for Connection: how far the connection got with the view's action beside it, the Settings card (the
 * entry's form or its JSON, what the server negotiated, the save bar), then Remove in a danger card. `children`
 * sits between the stage track and the Settings card. */
export function ConnectionView({
  host,
  server,
  draft,
  onDraft,
  errors,
  setDirty,
  stored,
  action,
  children,
  onSave,
  onDiscard,
  onRemove,
  saving,
  blocked,
}: {
  api: Api
  host: Host
  server: Server | null
  draft: Draft
  onDraft: (d: Draft) => void
  errors: Issue[]
  setDirty: (dirty: boolean) => void
  stored: unknown
  /** Sign in, Sign out or Turn on: beside the stage track, where the connection stopped. */
  action?: ReactNode
  children?: ReactNode
  onSave: () => void
  onDiscard: () => void
  onRemove: (() => void) | null
  saving: boolean
  blocked: string | null
}) {
  // The JSON view's text and why it is not an entry; null while the form shows.
  const [json, setJson] = useState<{ text: string; error: string | null } | null>(null)
  // The draft the JSON text shows: any other draft reaching the open view is a reload, and rewrites the text.
  const shown = useRef(draft)
  const removeId = useId()
  const SecretKeyField = host.components?.SecretKeyField as ComponentType<SecretKeyFieldProps> | undefined
  const fe = fieldErrors(errors, draft.name)
  const set = (change: Partial<Draft>) => onDraft({ ...draft, ...change })
  // Refused JSON text is an edit the draft does not hold yet.
  const dirty = draftDirty(draft, stored) || Boolean(json?.error)

  useEffect(() => setDirty(dirty), [dirty, setDirty])
  // Refused text leaves with the view: the page's dirty state is then the draft's alone.
  useEffect(() => () => setDirty(false), [setDirty])
  useEffect(() => {
    if (draft === shown.current) return
    shown.current = draft
    setJson((open) => (open && !open.error ? { text: jsonText(draft), error: null } : open))
  }, [draft])

  const field = (
    path: string,
    label: string,
    control: (props: SettingsFieldControlProps) => ReactNode,
    description?: string,
    className?: string,
  ) => (
    <SettingsField
      className={className}
      field={path}
      label={label}
      description={description}
      error={fe[path]}
      layout="stacked"
      controlSize="full"
      renderControl={control}
    />
  )
  const text = (path: string, label: string, value: string, onChange: (next: string) => void, description?: string) =>
    field(path, label, (props) => <Input {...props} value={value} onChange={onChange} />, description)
  // With the console's SecretKeyField the card is the row and carries its own label.
  const secret = (path: string, label: string, vault: string, value: string, onChange: (next: string) => void) => {
    const control = (props?: SettingsFieldControlProps) => (
      <SecretValue
        {...props}
        SecretKeyField={SecretKeyField}
        vault={vault}
        label={label}
        value={value}
        onChange={onChange}
        disabled={draft.name === ''}
      />
    )
    if (!SecretKeyField) return field(path, label, control)
    return (
      // biome-ignore lint/a11y/useSemanticElements: SettingsList is a div list, so its rows are div list items
      <div role="listitem" data-field={path}>
        {control()}
        {fe[path] ? <p className="mcp-ui-input-error">{fe[path]}</p> : null}
      </div>
    )
  }
  const rowErrors = (prefix: string, names: string[]) =>
    Object.fromEntries(names.flatMap((name, i) => (fe[`${prefix}/${name}`] ? [[i, fe[`${prefix}/${name}`]]] : [])))
  const keyValues = (path: 'headers' | 'env', kind: 'header' | 'env', label: string) =>
    field(path, label, () => (
      <KeyValueRows
        kind={kind}
        rows={draft[path]}
        onChange={(rows) => set(path === 'headers' ? { headers: rows } : { env: rows })}
        server={draft.name}
        SecretKeyField={SecretKeyField}
        errors={rowErrors(
          path,
          draft[path].map((row) => row.name),
        )}
      />
    ))
  const args = () => {
    const errs = rowErrors(
      'args',
      draft.args.map((_, i) => String(i)),
    )
    return field('args', 'Arguments', () => (
      <fieldset className="mcp-ui-kv" aria-label="Arguments">
        {draft.args.map((arg, i) => (
          <div key={i} className="mcp-ui-kv-row">
            <Input
              aria-label={`Argument ${i + 1}`}
              aria-invalid={errs[i] ? true : undefined}
              value={arg}
              onChange={(next) => set({ args: draft.args.map((a, j) => (j === i ? next : a)) })}
            />
            <IconButton
              variant="ghost"
              label={`Remove argument ${i + 1}`}
              onClick={() => set({ args: draft.args.filter((_, j) => j !== i) })}
            >
              <X size={16} aria-hidden />
            </IconButton>
            {errs[i] ? <p className="mcp-ui-input-error">{errs[i]}</p> : null}
          </div>
        ))}
        <Button variant="ghost" size="sm" onClick={() => set({ args: [...draft.args, ''] })}>
          <Plus size={16} aria-hidden />
          Add argument
        </Button>
      </fieldset>
    ))
  }

  const { bearer, clientCredentials: cc, oauth } = draft
  const form = (
    <>
      <SettingsList className="mcp-ui-settings">
        {field(
          'name',
          'Name',
          (props) => (
            <Input {...props} value={draft.name} readOnly={server !== null} onChange={(name) => set({ name })} />
          ),
          undefined,
          'mcp-ui-half',
        )}
        {field(
          'type',
          'Transport',
          () => (
            <SegmentedControl
              variant="radio"
              aria-label="Transport"
              value={draft.transport}
              options={TRANSPORTS}
              onChange={(transport) => set({ transport })}
            />
          ),
          undefined,
          'mcp-ui-half',
        )}
        {draft.transport === 'http' ? (
          <>
            {text('url', 'URL', draft.url, (url) => set({ url }))}
            {keyValues('headers', 'header', 'Headers')}
          </>
        ) : (
          <>
            {text('command', 'Command', draft.command, (command) => set({ command }))}
            {args()}
            {keyValues('env', 'env', 'Environment')}
            {text('cwd', 'Working directory', draft.cwd, (cwd) => set({ cwd }))}
          </>
        )}
      </SettingsList>
      {draft.transport === 'http' ? (
        <SettingsList className="mcp-ui-settings">
          {field('auth', 'Authentication', () => (
            <SegmentedControl
              variant="radio"
              aria-label="Authentication"
              value={draft.authMode}
              options={AUTH_MODES}
              onChange={(authMode) => set({ authMode })}
            />
          ))}
          {draft.authMode === 'bearer'
            ? secret('auth/token', 'API key', secretVaultName(draft.name, 'TOKEN'), bearer.token, (token) =>
                set({ bearer: { token } }),
              )
            : null}
          {draft.authMode === 'client_credentials' ? (
            <>
              {text('auth/token_url', 'Token URL', cc.token_url, (token_url) =>
                set({ clientCredentials: { ...cc, token_url } }),
              )}
              {text('auth/client_id', 'Client ID', cc.client_id, (client_id) =>
                set({ clientCredentials: { ...cc, client_id } }),
              )}
              {secret(
                'auth/client_secret',
                'Client secret',
                secretVaultName(draft.name, 'CLIENT_SECRET'),
                cc.client_secret,
                (client_secret) => set({ clientCredentials: { ...cc, client_secret } }),
              )}
              {text(
                'auth/scope',
                'Scope',
                cc.scope,
                (scope) => set({ clientCredentials: { ...cc, scope } }),
                'Sent as written. Linear wants commas.',
              )}
            </>
          ) : null}
          {draft.authMode === 'oauth'
            ? text(
                'auth/scopes',
                'Scopes',
                oauth.scopes,
                (scopes) => set({ oauth: { ...oauth, scopes } }),
                'Leave empty to use what the server asks for.',
              )
            : null}
        </SettingsList>
      ) : null}
      {draft.transport === 'http' && draft.authMode === 'oauth' ? (
        <details
          className="mcp-ui-optional"
          open={
            Boolean(fe['auth/client_id'] || fe['auth/client_secret'] || fe['auth/client_metadata_url']) || undefined
          }
        >
          <summary>Pre-registered client</summary>
          <SettingsList className="mcp-ui-settings">
            {text('auth/client_id', 'Client ID', oauth.client_id, (client_id) =>
              set({ oauth: { ...oauth, client_id } }),
            )}
            {secret(
              'auth/client_secret',
              'Client secret',
              secretVaultName(draft.name, 'CLIENT_SECRET'),
              oauth.client_secret,
              (client_secret) => set({ oauth: { ...oauth, client_secret } }),
            )}
            {text('auth/client_metadata_url', 'Client metadata URL', oauth.client_metadata_url, (client_metadata_url) =>
              set({ oauth: { ...oauth, client_metadata_url } }),
            )}
          </SettingsList>
        </details>
      ) : null}
    </>
  )

  const editJson = (value: string) => {
    const read = readEntryEdit(draft, value)
    if (read.draft) {
      shown.current = read.draft
      onDraft(read.draft)
    }
    setJson({ text: value, error: read.error })
  }
  // The refused text goes too: the draft the page puts back rewrites it.
  const discard = () => {
    setJson((open) => (open ? { ...open, error: null } : open))
    onDiscard()
  }
  // The errors no field shows, with their paths: a refused save is never silent.
  const unplaced = unplacedErrors(fe, json ? 'json' : 'form')
  const issues =
    unplaced.length > 0 ? (
      <ul className="mcp-ui-issues" role="alert">
        {unplaced.map((issue) => (
          <li key={issue.path}>
            <code>{issue.path}</code>
            <span>{issue.message}</span>
          </li>
        ))}
      </ul>
    ) : null

  const bar = dirty || saving
  const facts = server?.protocol_version
    ? [
        { label: 'Protocol', value: server.protocol_version },
        {
          label: 'Server',
          value:
            server.server_info &&
            `${server.server_info.title ?? server.server_info.name} ${server.server_info.version}`,
        },
        { label: 'Advertises', value: advertisesText(server) },
      ].filter((item) => item.value)
    : null
  const [removeLine] = server ? removeLines(server, 0) : []

  return (
    <div className="mcp-ui-detail">
      <div className="mcp-ui-track">
        <ol className="mcp-ui-stages" aria-label="Connection stages">
          {stageTrack(server).map(({ id, tone, pulse, label }) => (
            <li
              key={id}
              className="mcp-ui-stage"
              data-tone={tone}
              aria-current={id === server?.stage ? 'step' : undefined}
            >
              {/* A stage passed reads as done; the one it stopped at, and those ahead, keep their dot. */}
              {tone === 'ok' ? (
                <CircleCheck size={16} aria-hidden />
              ) : (
                <StatusDot className="mcp-ui-stage-dot" tone={tone} pulse={pulse} />
              )}
              {label}
            </li>
          ))}
        </ol>
        {action}
      </div>
      {children}
      <section className="mcp-ui-card mcp-ui-form" aria-label="Entry">
        <div className="mcp-ui-card-head">
          <h3 className="mcp-ui-card-title">Settings</h3>
          <SegmentedControl
            variant="radio"
            aria-label="Entry view"
            value={json ? 'json' : 'form'}
            options={MODES}
            onChange={(mode) => {
              if (!json?.error) setJson(mode === 'json' ? { text: jsonText(draft), error: null } : null)
            }}
          />
        </div>
        <div className="mcp-ui-card-body">
          {fe[''] ? <StatusPanel variant="alert" headline={fe['']} /> : null}
          {json ? (
            <>
              <CodeEditor
                className="mcp-ui-payload-json"
                language="json"
                value={json.text}
                onChange={editJson}
                aria-label="Entry as JSON"
              />
              {json.error ? <p className="mcp-ui-input-error">{json.error}</p> : null}
              {issues}
            </>
          ) : (
            <>
              {issues}
              {form}
            </>
          )}
        </div>
        {facts || bar ? (
          // Sticky while the save bar shows: Save stays in reach while a long form scrolls.
          <div className="mcp-ui-card-foot" data-sticky={bar || undefined}>
            {facts ? <MetaRow className="mcp-ui-facts" items={facts} /> : null}
            {bar ? (
              <SaveBar
                onSave={onSave}
                onDiscard={discard}
                saving={saving}
                blocked={json?.error ? 'Fix the JSON first' : blocked}
              />
            ) : null}
          </div>
        ) : null}
      </section>
      {onRemove && server ? (
        <section className="mcp-ui-card mcp-ui-danger-card" aria-labelledby={removeId}>
          <div>
            <h3 id={removeId} className="mcp-ui-card-title">
              Remove server
            </h3>
            {removeLine ? <p className="mcp-ui-quiet-line">{removeLine}</p> : null}
          </div>
          <Button variant="secondary" size="sm" className="mcp-ui-danger" onClick={onRemove}>
            Remove {server.name}
          </Button>
        </section>
      ) : null}
    </div>
  )
}
