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
import { Plus, X } from 'lucide-react'
import { type ComponentType, type ReactNode, useEffect, useState } from 'react'
import type { Api } from './api'
import { type Draft, draftDirty, draftFromEntry, entryFromDraft, fieldErrors, secretVaultName } from './config-model'
import { KeyValueRows, SecretValue } from './KeyValueRows'
import { advertisesText, stageTrack } from './presentation'
import { parsePayload } from './schema-form'
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

const pretty = (value: unknown) => JSON.stringify(value, null, 2)

/** Column 3 for Connection: how far the connection got, the entry's form (or its JSON), and what the server
 * negotiated. `children` sits between the stage track and the form. */
export function ConnectionView({
  host,
  server,
  draft,
  onDraft,
  errors,
  setDirty,
  stored,
  children,
}: {
  api: Api
  host: Host
  server: Server | null
  draft: Draft
  onDraft: (d: Draft) => void
  errors: Issue[]
  setDirty: (dirty: boolean) => void
  stored: unknown
  children?: ReactNode
}) {
  // The JSON view's text and why it is not an entry; null while the form shows.
  const [json, setJson] = useState<{ text: string; error: string | null } | null>(null)
  const SecretKeyField = host.components?.SecretKeyField as ComponentType<SecretKeyFieldProps> | undefined
  const fe = fieldErrors(errors, draft.name)
  const set = (change: Partial<Draft>) => onDraft({ ...draft, ...change })

  useEffect(() => setDirty(draftDirty(draft, stored)), [draft, stored, setDirty])

  const field = (
    path: string,
    label: string,
    control: (props: SettingsFieldControlProps) => ReactNode,
    description?: string,
  ) => (
    <SettingsField
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
      <div role="listitem" className="mcp-ui-form-card" data-field={path}>
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
      <SettingsList>
        {field('name', 'Name', (props) => (
          <Input {...props} value={draft.name} readOnly={server !== null} onChange={(name) => set({ name })} />
        ))}
        {field('type', 'Transport', () => (
          <SegmentedControl
            variant="radio"
            aria-label="Transport"
            value={draft.transport}
            options={TRANSPORTS}
            onChange={(transport) => set({ transport })}
          />
        ))}
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
        <SettingsList>
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
        <details className="mcp-ui-optional">
          <summary>Pre-registered client</summary>
          <SettingsList>
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
    const parsed = parsePayload(value)
    if ('payload' in parsed) onDraft(draftFromEntry(draft.name, parsed.payload))
    setJson({ text: value, error: 'error' in parsed ? parsed.error : null })
  }

  return (
    <div className="mcp-ui-detail">
      <ol className="mcp-ui-stages" aria-label="Connection stages">
        {stageTrack(server).map((stage) => (
          <li
            key={stage.id}
            className="mcp-ui-stage"
            data-tone={stage.tone}
            aria-current={stage.id === server?.stage ? 'step' : undefined}
          >
            <StatusDot tone={stage.tone} pulse={stage.pulse} />
            {stage.label}
          </li>
        ))}
      </ol>
      {children}
      <section className="mcp-ui-form" aria-label="Entry">
        <SegmentedControl
          variant="radio"
          aria-label="Entry view"
          value={json ? 'json' : 'form'}
          options={MODES}
          onChange={(mode) => {
            if (!json?.error) setJson(mode === 'json' ? { text: pretty(entryFromDraft(draft)), error: null } : null)
          }}
        />
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
          </>
        ) : (
          form
        )}
      </section>
      {server?.protocol_version ? (
        <MetaRow
          className="mcp-ui-facts"
          items={[
            { label: 'Protocol', value: server.protocol_version },
            {
              label: 'Server',
              value:
                server.server_info &&
                `${server.server_info.title ?? server.server_info.name} ${server.server_info.version}`,
            },
            { label: 'Advertises', value: advertisesText(server) },
          ].filter((item) => item.value)}
        />
      ) : null}
    </div>
  )
}
