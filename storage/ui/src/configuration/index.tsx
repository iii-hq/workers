/**
 * Purpose-built configuration editor for the storage worker. The Console
 * keeps ownership of dirty tracking, schema validation, save, reset, and the
 * unsaved-change guard; this component only edits the JSON draft.
 */

import {
  Button,
  type ConfigFormProps,
  Input,
  type JsonValue,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import {
  BackIcon,
  BucketIcon,
  ChevronIcon,
  StorageIcon,
  TrashIcon,
  useContainerNarrow,
} from '../widgets'

type JsonObject = { [key: string]: JsonValue }
type ProviderName = 'local' | 's3' | 'gcs' | 'r2'
type Selection = { kind: 'local' } | { kind: 'bucket'; name: string }

const CONFIG_NARROW_BELOW = 620
const PROVIDERS: ProviderName[] = ['local', 's3', 'gcs', 'r2']

function isObject(value: JsonValue | undefined): value is JsonObject {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function asObject(value: JsonValue | undefined): JsonObject {
  return isObject(value) ? { ...value } : {}
}

function asString(value: JsonValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

function pointer(...segments: string[]) {
  return `/${segments.map((segment) => segment.replaceAll('~', '~0').replaceAll('/', '~1')).join('/')}`
}

function fieldError(errors: ConfigFormProps['errors'], ...segments: string[]) {
  return errors?.get(pointer(...segments))
}

function providerOf(bucket: JsonObject): ProviderName {
  const provider = asString(bucket.provider)
  return PROVIDERS.includes(provider as ProviderName) ? (provider as ProviderName) : 'local'
}

function providerDefaults(provider: ProviderName): JsonObject {
  switch (provider) {
    case 's3':
      return { provider: 's3', region: 'us-east-1' }
    case 'gcs':
      return { provider: 'gcs' }
    case 'r2':
      return {
        provider: 'r2',
        account_id: '',
        access_key_id: '',
        secret_access_key: '',
      }
    case 'local':
      return { provider: 'local' }
  }
}

function validBucketName(name: string) {
  return /^[a-z0-9][a-z0-9_-]{0,62}$/.test(name)
}

function nextBucketName(buckets: JsonObject) {
  if (buckets.bucket === undefined) return 'bucket'
  let suffix = 2
  while (buckets[`bucket-${suffix}`] !== undefined) suffix += 1
  return `bucket-${suffix}`
}

function Field({
  label,
  hint,
  error,
  className = '',
  children,
}: {
  label: ReactNode
  hint?: ReactNode
  error?: string
  className?: string
  children: ReactNode
}) {
  return (
    <div className={`storage-cfg-field${className ? ` ${className}` : ''}`}>
      <div className="storage-cfg-field-label">
        <span>{label}</span>
      </div>
      {children}
      {error ? <p className="storage-cfg-error" role="alert">{error}</p> : null}
      {hint ? <p className="storage-cfg-hint">{hint}</p> : null}
    </div>
  )
}

function TextField({
  id,
  path,
  label,
  value,
  placeholder,
  hint,
  error,
  type = 'text',
  onChange,
}: {
  id: string
  path: string
  label: string
  value: string
  placeholder?: string
  hint?: ReactNode
  error?: string
  type?: 'text' | 'password'
  onChange: (value: string) => void
}) {
  const [revealed, setRevealed] = useState(false)
  const secret = type === 'password'
  return (
    <Field label={<label htmlFor={id}>{label}</label>} hint={hint} error={error}>
      <div className="storage-cfg-input-row">
        <Input
          id={id}
          data-field={path}
          className="storage-cfg-input"
          name={id}
          type={secret && !revealed ? 'password' : 'text'}
          value={value}
          preserveCase
          spellCheck={false}
          autoComplete="off"
          placeholder={placeholder}
          onChange={onChange}
        />
        {secret ? (
          <button
            type="button"
            className="storage-cfg-reveal"
            aria-label={`${revealed ? 'Hide' : 'Show'} ${label}`}
            onClick={() => setRevealed((current) => !current)}
          >
            {revealed ? 'hide' : 'show'}
          </button>
        ) : null}
      </div>
    </Field>
  )
}

function SelectField({
  id,
  path,
  label,
  value,
  options,
  hint,
  error,
  onChange,
}: {
  id: string
  path: string
  label: string
  value: string
  options: Array<{ value: string; label: string }>
  hint?: ReactNode
  error?: string
  onChange: (value: string) => void
}) {
  return (
    <Field label={<label htmlFor={id}>{label}</label>} hint={hint} error={error}>
      <div className="storage-cfg-select-wrap">
        <select
          id={id}
          data-field={path}
          className="storage-cfg-select"
          name={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>{option.label}</option>
          ))}
        </select>
        <ChevronIcon className="storage-cfg-select-icon" />
      </div>
    </Field>
  )
}

function CheckField({
  id,
  path,
  label,
  hint,
  checked,
  onChange,
}: {
  id: string
  path: string
  label: string
  hint?: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="storage-cfg-check-field">
      <label className="storage-cfg-check-row" htmlFor={id}>
        <input
          id={id}
          data-field={path}
          name={id}
          type="checkbox"
          checked={checked}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span>{label}</span>
      </label>
      {hint ? <p className="storage-cfg-hint">{hint}</p> : null}
    </div>
  )
}

function ConfigNav({
  buckets,
  selection,
  localHttpEnabled,
  onSelect,
  onAdd,
}: {
  buckets: JsonObject
  selection: Selection
  localHttpEnabled: boolean
  onSelect: (selection: Selection) => void
  onAdd: () => void
}) {
  const names = Object.keys(buckets).sort((left, right) => left.localeCompare(right))
  return (
    <nav className="storage-cfg-nav" aria-label="Storage configuration sections">
      <div className="storage-cfg-nav-group">
        <p className="storage-cfg-nav-label">runtime</p>
        <button
          type="button"
          className={`storage-cfg-nav-row${selection.kind === 'local' ? ' active' : ''}`}
          aria-current={selection.kind === 'local' ? 'page' : undefined}
          onClick={() => onSelect({ kind: 'local' })}
        >
          <StorageIcon className="storage-cfg-nav-icon" />
          <span className="storage-cfg-nav-copy">
            <span className="storage-cfg-nav-name">local storage</span>
            <span className="storage-cfg-nav-meta">HTTP {localHttpEnabled ? 'enabled' : 'disabled'}</span>
          </span>
          <ChevronIcon className="storage-cfg-nav-chevron" />
        </button>
      </div>
      <div className="storage-cfg-nav-group buckets">
        <div className="storage-cfg-nav-heading">
          <p className="storage-cfg-nav-label">buckets</p>
          <span>{names.length}</span>
        </div>
        {names.length === 0 ? (
          <p className="storage-cfg-nav-empty">No buckets configured.</p>
        ) : (
          <ul role="list" className="storage-cfg-nav-list">
            {names.map((name) => {
              const provider = providerOf(asObject(buckets[name]))
              const active = selection.kind === 'bucket' && selection.name === name
              return (
                <li key={name}>
                  <button
                    type="button"
                    className={`storage-cfg-nav-row${active ? ' active' : ''}`}
                    aria-current={active ? 'page' : undefined}
                    onClick={() => onSelect({ kind: 'bucket', name })}
                  >
                    <BucketIcon className="storage-cfg-nav-icon" />
                    <span className="storage-cfg-nav-copy">
                      <span className="storage-cfg-nav-name">{name}</span>
                      <span className="storage-cfg-nav-meta">{provider}</span>
                    </span>
                    <ChevronIcon className="storage-cfg-nav-chevron" />
                  </button>
                </li>
              )
            })}
          </ul>
        )}
      </div>
      <div className="storage-cfg-nav-action">
        <Button variant="ghost" size="sm" onClick={onAdd}>+ add bucket</Button>
      </div>
    </nav>
  )
}

function EditorHeader({
  icon,
  title,
  description,
  narrow,
  onBack,
  actions,
}: {
  icon: ReactNode
  title: string
  description: string
  narrow: boolean
  onBack: () => void
  actions?: ReactNode
}) {
  return (
    <header className="storage-cfg-editor-head">
      {narrow ? (
        <button type="button" className="storage-cfg-back" onClick={onBack} aria-label="Back to configuration sections">
          <BackIcon />
        </button>
      ) : null}
      <span className="storage-cfg-editor-icon">{icon}</span>
      <div className="storage-cfg-editor-title">
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
      {actions ? <div className="storage-cfg-editor-actions">{actions}</div> : null}
    </header>
  )
}

function LocalEditor({
  value,
  errors,
  narrow,
  onBack,
  onChange,
}: {
  value: JsonObject
  errors: ConfigFormProps['errors']
  narrow: boolean
  onBack: () => void
  onChange: (next: JsonObject) => void
}) {
  const providers = asObject(value.providers)
  const local = asObject(providers.local)
  const httpEnabled = isObject(local.http)
  const http = asObject(local.http)

  const commitLocal = (nextLocal: JsonObject) => {
    const nextProviders = { ...providers }
    if (Object.keys(nextLocal).length > 0) nextProviders.local = nextLocal
    else delete nextProviders.local
    const next = { ...value }
    if (Object.keys(nextProviders).length > 0) next.providers = nextProviders
    else delete next.providers
    onChange(next)
  }

  const setLocalString = (field: string, raw: string) => {
    const next = { ...local }
    if (raw === '') delete next[field]
    else next[field] = raw
    commitLocal(next)
  }

  const setHttpString = (field: string, raw: string) => {
    const nextHttp = { ...http }
    if (raw === '') delete nextHttp[field]
    else nextHttp[field] = raw
    commitLocal({ ...local, http: nextHttp })
  }

  return (
    <section className="storage-cfg-editor" data-section="providers/local" tabIndex={-1}>
      <EditorHeader
        icon={<StorageIcon />}
        title="local storage"
        description="Filesystem persistence and direct browser transfers."
        narrow={narrow}
        onBack={onBack}
      />
      <div className="storage-cfg-editor-scroll">
        <section className="storage-cfg-section">
          <div className="storage-cfg-section-head">
            <div>
              <h4>Filesystem</h4>
              <p>Objects and metadata are written directly by this worker.</p>
            </div>
          </div>
          <TextField
            id="storage-cfg-local-data-dir"
            path="providers/local/data_dir"
            label="Data directory"
            value={asString(local.data_dir)}
            placeholder="data/storage"
            hint="Relative paths resolve from the worker process directory. Existing RustFS data is not imported automatically."
            error={fieldError(errors, 'providers', 'local', 'data_dir')}
            onChange={(next) => setLocalString('data_dir', next)}
          />
        </section>

        <section className="storage-cfg-section">
          <div className="storage-cfg-section-head">
            <div>
              <h4>Direct transfers</h4>
              <p>Serve signed uploads and downloads without routing file bytes through RPC.</p>
            </div>
          </div>
          <CheckField
            id="storage-cfg-local-http"
            path="providers/local/http"
            label="Enable the HTTP transfer server"
            hint="Required by presignPost and local signed GET/PUT URLs. Inline getObject and putObject still work when disabled."
            checked={httpEnabled}
            onChange={(checked) => {
              if (checked) {
                commitLocal({ ...local, http: { bind_address: '127.0.0.1:0' } })
              } else {
                const next = { ...local }
                delete next.http
                commitLocal(next)
              }
            }}
          />
          {httpEnabled ? (
            <div className="storage-cfg-field-grid">
              <TextField
                id="storage-cfg-local-bind"
                path="providers/local/http/bind_address"
                label="Bind address"
                value={asString(http.bind_address)}
                placeholder="127.0.0.1:0"
                hint="Use 0.0.0.0 with a fixed port to listen on LAN, VPN, or container interfaces."
                error={fieldError(errors, 'providers', 'local', 'http', 'bind_address')}
                onChange={(next) => setHttpString('bind_address', next)}
              />
              <TextField
                id="storage-cfg-local-public-url"
                path="providers/local/http/public_url"
                label="Public URL"
                value={asString(http.public_url)}
                placeholder="http://10.0.0.42:49200"
                hint="The browser-visible URL returned in signatures; use the VPN, proxy, or mapped address here."
                error={fieldError(errors, 'providers', 'local', 'http', 'public_url')}
                onChange={(next) => setHttpString('public_url', next)}
              />
            </div>
          ) : (
            <div className="storage-cfg-disabled-note">
              Direct transfer is off. Local buckets remain available for genuinely small inline values only.
            </div>
          )}
        </section>
      </div>
    </section>
  )
}

function NotificationFields({
  name,
  provider,
  bucket,
  errors,
  onChange,
}: {
  name: string
  provider: ProviderName
  bucket: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (next: JsonObject) => void
}) {
  if (provider === 'local') return null
  const notificationsEnabled = isObject(bucket.notifications)
  const notifications = asObject(bucket.notifications)
  const defaults: Record<Exclude<ProviderName, 'local'>, JsonObject> = {
    s3: { sqs_queue_url: '' },
    gcs: { pubsub_subscription: '' },
    r2: { queue_id: '', api_token: '' },
  }
  const setNotification = (field: string, raw: string) => {
    onChange({ ...bucket, notifications: { ...notifications, [field]: raw } })
  }

  return (
    <section className="storage-cfg-section">
      <div className="storage-cfg-section-head">
        <div>
          <h4>Object events</h4>
          <p>Connect provider notifications to object-created and object-deleted triggers.</p>
        </div>
      </div>
      <CheckField
        id={`storage-cfg-${name}-notifications`}
        path={`buckets/${name}/notifications`}
        label="Enable object event notifications"
        hint="Saving starts or stops the provider poller immediately. Existing trigger registrations remain intact."
        checked={notificationsEnabled}
        onChange={(checked) => {
          const next = { ...bucket }
          if (checked) next.notifications = defaults[provider]
          else delete next.notifications
          onChange(next)
        }}
      />
      {notificationsEnabled && provider === 's3' ? (
        <TextField
          id={`storage-cfg-${name}-sqs`}
          path={`buckets/${name}/notifications/sqs_queue_url`}
          label="SQS queue URL"
          value={asString(notifications.sqs_queue_url)}
          placeholder="https://sqs.us-east-1.amazonaws.com/123/events"
          hint="The bucket must publish ObjectCreated and ObjectRemoved events to this queue. Saving reconnects the poller with the current region."
          error={fieldError(errors, 'buckets', name, 'notifications', 'sqs_queue_url')}
          onChange={(next) => setNotification('sqs_queue_url', next)}
        />
      ) : null}
      {notificationsEnabled && provider === 'gcs' ? (
        <TextField
          id={`storage-cfg-${name}-pubsub`}
          path={`buckets/${name}/notifications/pubsub_subscription`}
          label="Pub/Sub subscription"
          value={asString(notifications.pubsub_subscription)}
          placeholder="projects/my-project/subscriptions/storage-events"
          error={fieldError(errors, 'buckets', name, 'notifications', 'pubsub_subscription')}
          onChange={(next) => setNotification('pubsub_subscription', next)}
        />
      ) : null}
      {notificationsEnabled && provider === 'r2' ? (
        <div className="storage-cfg-field-grid">
          <TextField
            id={`storage-cfg-${name}-queue-id`}
            path={`buckets/${name}/notifications/queue_id`}
            label="Cloudflare queue ID"
            value={asString(notifications.queue_id)}
            placeholder="queue-id"
            error={fieldError(errors, 'buckets', name, 'notifications', 'queue_id')}
            onChange={(next) => setNotification('queue_id', next)}
          />
          <TextField
            id={`storage-cfg-${name}-queue-token`}
            path={`buckets/${name}/notifications/api_token`}
            label="Queue API token"
            value={asString(notifications.api_token)}
            placeholder="${R2_QUEUE_API_TOKEN}"
            hint="Prefer an environment reference so exports never contain the token."
            error={fieldError(errors, 'buckets', name, 'notifications', 'api_token')}
            type="password"
            onChange={(next) => setNotification('api_token', next)}
          />
        </div>
      ) : null}
    </section>
  )
}

function BucketEditor({
  name,
  bucket,
  nameDraft,
  nameError,
  errors,
  narrow,
  onBack,
  onNameDraft,
  onRename,
  onChange,
  onRemove,
}: {
  name: string
  bucket: JsonObject
  nameDraft: string
  nameError?: string
  errors: ConfigFormProps['errors']
  narrow: boolean
  onBack: () => void
  onNameDraft: (value: string) => void
  onRename: () => void
  onChange: (next: JsonObject) => void
  onRemove: () => void
}) {
  const provider = providerOf(bucket)
  const setOptional = (field: string, raw: string) => {
    const next = { ...bucket }
    if (raw === '') delete next[field]
    else next[field] = raw
    onChange(next)
  }
  const setRequired = (field: string, raw: string) => onChange({ ...bucket, [field]: raw })

  return (
    <section className="storage-cfg-editor" data-section={`buckets/${name}`} tabIndex={-1}>
      <EditorHeader
        icon={<BucketIcon />}
        title={name}
        description={`${provider} bucket exposed to workers as bucket:${name}.`}
        narrow={narrow}
        onBack={onBack}
        actions={(
          <Button variant="ghost" size="sm" onClick={onRemove} aria-label={`Remove ${name}`}>
            <TrashIcon className="storage-cfg-button-icon" /> remove
          </Button>
        )}
      />
      <div className="storage-cfg-editor-scroll">
        <section className="storage-cfg-section">
          <div className="storage-cfg-section-head">
            <div>
              <h4>Bucket identity</h4>
              <p>The worker-facing name and provider determine request routing.</p>
            </div>
          </div>
          <div className="storage-cfg-field-grid">
            <Field
              label={<label htmlFor={`storage-cfg-${name}-name`}>Worker bucket name</label>}
              hint="Lowercase letters, numbers, underscores, and hyphens; maximum 63 characters."
              error={nameError ?? fieldError(errors, 'buckets', name)}
            >
              <Input
                id={`storage-cfg-${name}-name`}
                data-field={`buckets/${name}`}
                className="storage-cfg-input"
                name={`storage-cfg-${name}-name`}
                value={nameDraft}
                preserveCase
                spellCheck={false}
                onChange={onNameDraft}
                onBlur={onRename}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') event.currentTarget.blur()
                }}
              />
            </Field>
            <SelectField
              id={`storage-cfg-${name}-provider`}
              path={`buckets/${name}/provider`}
              label="Provider"
              value={provider}
              options={[
                { value: 'local', label: 'local — native filesystem' },
                { value: 's3', label: 's3 — AWS or compatible' },
                { value: 'gcs', label: 'gcs — Google Cloud Storage' },
                { value: 'r2', label: 'r2 — Cloudflare R2' },
              ]}
              onChange={(next) => onChange(providerDefaults(next as ProviderName))}
            />
          </div>
          <TextField
            id={`storage-cfg-${name}-underlying`}
            path={`buckets/${name}/bucket`}
            label="Provider bucket name"
            value={asString(bucket.bucket)}
            placeholder={name}
            hint="Optional override. Empty uses the worker bucket name."
            error={fieldError(errors, 'buckets', name, 'bucket')}
            onChange={(next) => setOptional('bucket', next)}
          />
        </section>

        {provider === 'local' ? (
          <section className="storage-cfg-section">
            <div className="storage-cfg-section-head">
              <div>
                <h4>Native local bucket</h4>
                <p>This bucket uses the filesystem and HTTP settings from local storage.</p>
              </div>
              <span className="storage-cfg-provider">local</span>
            </div>
            <div className="storage-cfg-info-note">
              Direct upload and download URLs are available only when the local HTTP transfer server is enabled in the runtime section.
            </div>
          </section>
        ) : null}

        {provider === 's3' ? (
          <>
            <section className="storage-cfg-section">
              <div className="storage-cfg-section-head">
                <div><h4>Connection</h4><p>AWS defaults apply when endpoint and static credentials are omitted.</p></div>
                <span className="storage-cfg-provider">s3</span>
              </div>
              <TextField
                id={`storage-cfg-${name}-region`}
                path={`buckets/${name}/region`}
                label="Region"
                value={asString(bucket.region)}
                placeholder="us-east-1"
                error={fieldError(errors, 'buckets', name, 'region')}
                onChange={(next) => setRequired('region', next)}
              />
              <div className="storage-cfg-field-grid">
                <TextField
                  id={`storage-cfg-${name}-access-key`}
                  path={`buckets/${name}/access_key_id`}
                  label="Access key ID"
                  value={asString(bucket.access_key_id)}
                  placeholder="${AWS_ACCESS_KEY_ID}"
                  error={fieldError(errors, 'buckets', name, 'access_key_id')}
                  onChange={(next) => setOptional('access_key_id', next)}
                />
                <TextField
                  id={`storage-cfg-${name}-secret-key`}
                  path={`buckets/${name}/secret_access_key`}
                  label="Secret access key"
                  value={asString(bucket.secret_access_key)}
                  placeholder="${AWS_SECRET_ACCESS_KEY}"
                  hint="Prefer an environment reference."
                  error={fieldError(errors, 'buckets', name, 'secret_access_key')}
                  type="password"
                  onChange={(next) => setOptional('secret_access_key', next)}
                />
              </div>
              <TextField
                id={`storage-cfg-${name}-session-token`}
                path={`buckets/${name}/session_token`}
                label="Session token"
                value={asString(bucket.session_token)}
                placeholder="Optional temporary credential"
                error={fieldError(errors, 'buckets', name, 'session_token')}
                type="password"
                onChange={(next) => setOptional('session_token', next)}
              />
              <TextField
                id={`storage-cfg-${name}-endpoint`}
                path={`buckets/${name}/endpoint_url`}
                label="Custom endpoint URL"
                value={asString(bucket.endpoint_url)}
                placeholder="http://minio:9000"
                hint="Use for MinIO, Ceph, SeaweedFS, LocalStack, or another S3-compatible service."
                error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                onChange={(next) => setOptional('endpoint_url', next)}
              />
              <CheckField
                id={`storage-cfg-${name}-path-style`}
                path={`buckets/${name}/force_path_style`}
                label="Force path-style addressing"
                hint="Usually required by S3-compatible services; AWS normally uses virtual-hosted addressing."
                checked={bucket.force_path_style === true}
                onChange={(checked) => {
                  const next = { ...bucket }
                  if (checked) next.force_path_style = true
                  else delete next.force_path_style
                  onChange(next)
                }}
              />
            </section>
            <NotificationFields name={name} provider={provider} bucket={bucket} errors={errors} onChange={onChange} />
          </>
        ) : null}

        {provider === 'gcs' ? (
          <>
            <section className="storage-cfg-section">
              <div className="storage-cfg-section-head">
                <div><h4>Connection</h4><p>Application Default Credentials are used when no file is configured.</p></div>
                <span className="storage-cfg-provider">gcs</span>
              </div>
              <TextField
                id={`storage-cfg-${name}-credentials-file`}
                path={`buckets/${name}/credentials_file`}
                label="Credentials file"
                value={asString(bucket.credentials_file)}
                placeholder="/etc/iii/gcs-service-account.json"
                error={fieldError(errors, 'buckets', name, 'credentials_file')}
                onChange={(next) => setOptional('credentials_file', next)}
              />
              <TextField
                id={`storage-cfg-${name}-endpoint`}
                path={`buckets/${name}/endpoint_url`}
                label="Custom endpoint URL"
                value={asString(bucket.endpoint_url)}
                placeholder="http://fake-gcs:4443"
                error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                onChange={(next) => setOptional('endpoint_url', next)}
              />
            </section>
            <NotificationFields name={name} provider={provider} bucket={bucket} errors={errors} onChange={onChange} />
          </>
        ) : null}

        {provider === 'r2' ? (
          <>
            <section className="storage-cfg-section">
              <div className="storage-cfg-section-head">
                <div><h4>Connection</h4><p>The standard endpoint is derived from the Cloudflare account ID.</p></div>
                <span className="storage-cfg-provider">r2</span>
              </div>
              <TextField
                id={`storage-cfg-${name}-account-id`}
                path={`buckets/${name}/account_id`}
                label="Account ID"
                value={asString(bucket.account_id)}
                placeholder="${R2_ACCOUNT_ID}"
                error={fieldError(errors, 'buckets', name, 'account_id')}
                onChange={(next) => setRequired('account_id', next)}
              />
              <div className="storage-cfg-field-grid">
                <TextField
                  id={`storage-cfg-${name}-access-key`}
                  path={`buckets/${name}/access_key_id`}
                  label="Access key ID"
                  value={asString(bucket.access_key_id)}
                  placeholder="${R2_ACCESS_KEY_ID}"
                  error={fieldError(errors, 'buckets', name, 'access_key_id')}
                  onChange={(next) => setRequired('access_key_id', next)}
                />
                <TextField
                  id={`storage-cfg-${name}-secret-key`}
                  path={`buckets/${name}/secret_access_key`}
                  label="Secret access key"
                  value={asString(bucket.secret_access_key)}
                  placeholder="${R2_SECRET_ACCESS_KEY}"
                  hint="Prefer an environment reference."
                  error={fieldError(errors, 'buckets', name, 'secret_access_key')}
                  type="password"
                  onChange={(next) => setRequired('secret_access_key', next)}
                />
              </div>
              <TextField
                id={`storage-cfg-${name}-endpoint`}
                path={`buckets/${name}/endpoint_url`}
                label="Custom endpoint URL"
                value={asString(bucket.endpoint_url)}
                placeholder="Optional R2-compatible endpoint"
                error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                onChange={(next) => setOptional('endpoint_url', next)}
              />
            </section>
            <NotificationFields name={name} provider={provider} bucket={bucket} errors={errors} onChange={onChange} />
          </>
        ) : null}
      </div>
    </section>
  )
}

export function StorageConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const providers = asObject(value.providers)
  const local = asObject(providers.local)
  const buckets = asObject(value.buckets)
  const names = Object.keys(buckets).sort((left, right) => left.localeCompare(right))
  const [rootRef, narrow] = useContainerNarrow(CONFIG_NARROW_BELOW)
  const [selection, setSelection] = useState<Selection>({ kind: 'local' })
  const [narrowPane, setNarrowPane] = useState<'nav' | 'editor'>('nav')
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({})
  const [nameErrors, setNameErrors] = useState<Record<string, string>>({})
  const domRef = useRef<HTMLDivElement | null>(null)
  const focusKey = props.focusField?.join('/') ?? ''

  const setRoot = useCallback((node: HTMLDivElement | null) => {
    rootRef(node)
    domRef.current = node
  }, [rootRef])

  const choose = (next: Selection) => {
    setSelection(next)
    setNarrowPane('editor')
  }

  useEffect(() => {
    if (selection.kind === 'bucket' && buckets[selection.name] === undefined) {
      setSelection({ kind: 'local' })
    }
  }, [selection, names.join('|')])

  useEffect(() => {
    const path = props.focusField
    if (!path || path.length === 0) return
    if (path[0] === 'buckets' && path[1] && buckets[path[1]] !== undefined) {
      setSelection({ kind: 'bucket', name: path[1] })
    } else if (path[0] === 'providers') {
      setSelection({ kind: 'local' })
    }
    setNarrowPane('editor')
  }, [focusKey])

  useEffect(() => {
    if (!focusKey || !domRef.current) return
    const target = domRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`)
      ?? domRef.current.querySelector<HTMLElement>(`[data-section="${CSS.escape(focusKey.split('/').slice(0, 2).join('/'))}"]`)
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [focusKey, selection.kind, selection.kind === 'bucket' ? selection.name : 'local'])

  const commitBuckets = (nextBuckets: JsonObject) => props.onChange({ ...value, buckets: nextBuckets })

  const addBucket = () => {
    const name = nextBucketName(buckets)
    commitBuckets({ ...buckets, [name]: providerDefaults('local') })
    setNameDrafts((current) => ({ ...current, [name]: name }))
    choose({ kind: 'bucket', name })
  }

  const renameBucket = (from: string) => {
    const raw = nameDrafts[from] ?? from
    const nextName = raw.trim()
    let error = ''
    if (!validBucketName(nextName)) {
      error = 'Use 1–63 characters: start with a lowercase letter or number, then only a–z, 0–9, _ or -.'
    } else if (nextName !== from && buckets[nextName] !== undefined) {
      error = `A bucket named ${nextName} already exists.`
    }
    if (error) {
      setNameErrors((current) => ({ ...current, [from]: error }))
      return
    }
    setNameErrors((current) => {
      const next = { ...current }
      delete next[from]
      return next
    })
    if (nextName === from) return
    const nextBuckets: JsonObject = {}
    for (const name of Object.keys(buckets)) {
      nextBuckets[name === from ? nextName : name] = buckets[name]
    }
    commitBuckets(nextBuckets)
    setNameDrafts((current) => {
      const next = { ...current, [nextName]: nextName }
      delete next[from]
      return next
    })
    setSelection({ kind: 'bucket', name: nextName })
  }

  const removeBucket = (name: string) => {
    if (!window.confirm(`Remove bucket ${name} from the storage configuration?`)) return
    const next = { ...buckets }
    delete next[name]
    commitBuckets(next)
    setSelection({ kind: 'local' })
    setNarrowPane(narrow ? 'nav' : 'editor')
  }

  const showNav = !narrow || narrowPane === 'nav'
  const showEditor = !narrow || narrowPane === 'editor'
  const selectedBucket = selection.kind === 'bucket' ? asObject(buckets[selection.name]) : null

  return (
    <div className={`storage-cfg${narrow ? ' narrow' : ''}`} ref={setRoot}>
      <div className="storage-cfg-workbench">
        {showNav ? (
          <ConfigNav
            buckets={buckets}
            selection={selection}
            localHttpEnabled={isObject(local.http)}
            onSelect={choose}
            onAdd={addBucket}
          />
        ) : null}
        {showEditor && selection.kind === 'local' ? (
          <LocalEditor value={value} errors={props.errors} narrow={narrow} onBack={() => setNarrowPane('nav')} onChange={props.onChange} />
        ) : null}
        {showEditor && selection.kind === 'bucket' && selectedBucket ? (
          <BucketEditor
            name={selection.name}
            bucket={selectedBucket}
            nameDraft={nameDrafts[selection.name] ?? selection.name}
            nameError={nameErrors[selection.name]}
            errors={props.errors}
            narrow={narrow}
            onBack={() => setNarrowPane('nav')}
            onNameDraft={(next) => setNameDrafts((current) => ({ ...current, [selection.name]: next }))}
            onRename={() => renameBucket(selection.name)}
            onChange={(next) => commitBuckets({ ...buckets, [selection.name]: next })}
            onRemove={() => removeBucket(selection.name)}
          />
        ) : null}
      </div>

      {props.errors && props.errors.size > 0 ? (
        <StatusPanel
          variant="alert"
          headline="Configuration needs attention"
          detail={`${props.errors.size} validation ${props.errors.size === 1 ? 'error is' : 'errors are'} marked in the form.`}
          className="storage-cfg-status"
        />
      ) : null}
    </div>
  )
}
