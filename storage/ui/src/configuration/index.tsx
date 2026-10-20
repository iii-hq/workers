/**
 * Purpose-built configuration editor for the storage worker. The Console
 * keeps ownership of dirty tracking, schema validation, save, reset, and the
 * unsaved-change guard; this component only edits the JSON draft.
 *
 * A bucket starts from its provider — local, Amazon S3, an S3-compatible
 * service (with presets), Cloudflare R2 or Google Cloud Storage — then its
 * icon and colour, its connection and its credentials. Credentials go to
 * the secrets worker the moment they are saved and the entry keeps
 * `secret://STORAGE_<BUCKET>_<FIELD>`; "Test bucket" asks the worker to
 * build the draft and list it. Mirrors BucketConfig (storage/src/config.rs).
 */

import {
  Button,
  Chip,
  type ConfigFormProps,
  Eyebrow,
  type Host,
  IconButton,
  Input,
  type JsonValue,
  List,
  ListItem,
  StatusDot,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { useContainerNarrow } from '@iii-dev/console-ui/hooks'
import { ChevronLeft, Database, Plus, Trash2 } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { BucketIcon } from '../lib/bucket-icons'
import { errorText, isMissingFunction } from '../lib/errors'
import { ColorPicker, IconPicker } from './BucketAppearance'
import {
  applyPreset,
  bucketDisplay,
  bucketNameError,
  bucketProblems,
  type CredentialField as CredentialFieldName,
  detectPreset,
  fieldValue,
  type JsonObject,
  newBucketEntry,
  nextBucketName,
  PROVIDER_CHOICES,
  type PresetId,
  type ProviderChoice,
  parseTestOutcome,
  presetById,
  providerChoiceOf,
  providerLabel,
  providerOf,
  S3_PRESETS,
  stageLabel,
  storeTarget,
  supportsSecretReferences,
  switchProvider,
  testBlocker,
  testBucketPayload,
  testOutcomeHint,
  testOutcomeText,
  underlyingBucket,
  withDisplay,
  withField,
  withRegion,
} from './bucket'
import { CredentialField } from './CredentialField'
import { CheckField, Field, SelectField, TextField } from './fields'
import { TestBucketRow, type TestState } from './TestBucketRow'

type Selection = { kind: 'local' } | { kind: 'bucket'; name: string }

/** What the running worker says about a saved bucket (`storage::listBuckets`). */
interface LiveBucket {
  error?: string
  error_stage?: string
}

const CONFIG_NARROW_BELOW = 620

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

/** The running worker's view of the saved buckets: which ones it could not build. */
function useLiveBuckets(host: Host) {
  const [live, setLive] = useState<Record<string, LiveBucket>>({})
  const [tick, setTick] = useState(0)
  useEffect(() => {
    let alive = true
    host.iii
      .trigger<{ buckets?: Array<{ name?: unknown; error?: unknown; error_stage?: unknown }> }>(
        'storage::listBuckets',
        {},
        { timeoutMs: 10_000 },
      )
      .then((response) => {
        if (!alive) return
        const next: Record<string, LiveBucket> = {}
        for (const row of response.buckets ?? []) {
          if (typeof row.name !== 'string') continue
          next[row.name] = {
            error: typeof row.error === 'string' && row.error ? row.error : undefined,
            error_stage: typeof row.error_stage === 'string' ? row.error_stage : undefined,
          }
        }
        setLive(next)
      })
      .catch(() => {
        // The form works without it; only the "as saved" notices go missing.
      })
    return () => {
      alive = false
    }
  }, [host, tick])
  return { live, refresh: () => setTick((n) => n + 1) }
}

function ConfigNav({
  buckets,
  live,
  selection,
  localHttpEnabled,
  onSelect,
  onAdd,
}: {
  buckets: JsonObject
  live: Record<string, LiveBucket>
  selection: Selection
  localHttpEnabled: boolean
  onSelect: (selection: Selection) => void
  onAdd: () => void
}) {
  const names = Object.keys(buckets).sort((left, right) => left.localeCompare(right))
  return (
    <nav className="storage-cfg-nav" aria-label="Storage configuration sections">
      <div className="storage-cfg-nav-group">
        <Eyebrow as="div" className="storage-cfg-nav-label">
          runtime
        </Eyebrow>
        <ListItem
          selected={selection.kind === 'local'}
          aria-current={selection.kind === 'local' ? 'page' : undefined}
          leading={<Database />}
          label="local storage"
          description={`HTTP ${localHttpEnabled ? 'enabled' : 'disabled'}`}
          onClick={() => onSelect({ kind: 'local' })}
        />
      </div>
      <div className="storage-cfg-nav-group buckets">
        <div className="storage-cfg-nav-heading">
          <Eyebrow as="div" className="storage-cfg-nav-label">
            buckets
          </Eyebrow>
          <span>{names.length}</span>
        </div>
        {names.length === 0 ? (
          <p className="storage-cfg-nav-empty">No buckets configured.</p>
        ) : (
          <List>
            {names.map((name) => {
              const bucket = asObject(buckets[name])
              const display = bucketDisplay(bucket)
              const active = selection.kind === 'bucket' && selection.name === name
              const error = live[name]?.error
              return (
                <ListItem
                  key={name}
                  selected={active}
                  aria-current={active ? 'page' : undefined}
                  leading={<BucketIcon name={display.icon} color={display.color} />}
                  label={name}
                  description={providerLabel(providerChoiceOf(bucket))}
                  trailing={
                    error ? (
                      <StatusDot tone="alert" title={error} aria-label="The worker could not build it" />
                    ) : undefined
                  }
                  onClick={() => onSelect({ kind: 'bucket', name })}
                />
              )
            })}
          </List>
        )}
      </div>
      <div className="storage-cfg-nav-action">
        <Button variant="ghost" size="sm" onClick={onAdd}>
          <Plus /> add bucket
        </Button>
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
        <IconButton label="Back to configuration sections" onClick={onBack}>
          <ChevronLeft />
        </IconButton>
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

function Section({
  title,
  description,
  chip,
  children,
}: {
  title: string
  description: string
  chip?: string
  children: ReactNode
}) {
  return (
    <section className="storage-cfg-section">
      <div className="storage-cfg-section-head">
        <div>
          <h4>{title}</h4>
          <p>{description}</p>
        </div>
        {chip ? <Chip>{chip}</Chip> : null}
      </div>
      {children}
    </section>
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
        icon={<Database />}
        title="local storage"
        description="Filesystem persistence and direct browser transfers."
        narrow={narrow}
        onBack={onBack}
      />
      <div className="storage-cfg-editor-scroll">
        <Section title="Filesystem" description="Objects and metadata are written directly by this worker.">
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
        </Section>

        <Section
          title="Direct transfers"
          description="Serve signed uploads and downloads without routing file bytes through RPC."
        >
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
        </Section>
      </div>
    </section>
  )
}

/** Shared by the bucket editor's sections: one credential field of this bucket. */
interface CredentialContext {
  host: Host
  name: string
  bucket: JsonObject
  buckets: JsonObject
  errors: ConfigFormProps['errors']
  referencesSupported: boolean | null
  onChange: (next: JsonObject) => void
  /**
   * One field of the entry as it is when the value arrives: a secret is
   * stored after an await, and the edits made meanwhile must survive it.
   */
  onField: (field: string, value: JsonValue | undefined) => void
}

function credential(
  ctx: CredentialContext,
  field: CredentialFieldName,
  label: string,
  options: {
    hint?: ReactNode
    mode?: 'secret' | 'identifier' | 'json'
    preferText?: boolean
    placeholder?: string
  } = {},
) {
  const value = fieldValue(ctx.bucket, field)
  const segments = field.split('.')
  return (
    <CredentialField
      key={field}
      host={ctx.host}
      id={`storage-cfg-${ctx.name}-${segments.join('-')}`}
      path={`buckets/${ctx.name}/${segments.join('/')}`}
      label={label}
      hint={options.hint}
      error={fieldError(ctx.errors, 'buckets', ctx.name, ...segments)}
      value={value}
      target={storeTarget(ctx.buckets, ctx.name, field, value)}
      bucket={ctx.name}
      mode={options.mode}
      preferText={options.preferText}
      referencesSupported={ctx.referencesSupported}
      placeholder={options.placeholder}
      onChange={(next) => ctx.onField(field, next)}
    />
  )
}

function NotificationFields({
  ctx,
  provider,
}: {
  ctx: CredentialContext
  provider: Exclude<ReturnType<typeof providerOf>, 'local'>
}) {
  const { name, bucket, errors, onChange } = ctx
  const notificationsEnabled = isObject(bucket.notifications)
  const notifications = asObject(bucket.notifications)
  const defaults: Record<typeof provider, JsonObject> = {
    s3: { sqs_queue_url: '' },
    gcs: { pubsub_subscription: '' },
    r2: { queue_id: '' },
  }
  const setNotification = (field: string, raw: string) => {
    onChange({ ...bucket, notifications: { ...notifications, [field]: raw } })
  }

  return (
    <Section
      title="Object events"
      description="Connect provider notifications to object-created and object-deleted triggers."
    >
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
        <>
          <TextField
            id={`storage-cfg-${name}-queue-id`}
            path={`buckets/${name}/notifications/queue_id`}
            label="Cloudflare queue ID"
            value={asString(notifications.queue_id)}
            placeholder="queue-id"
            error={fieldError(errors, 'buckets', name, 'notifications', 'queue_id')}
            onChange={(next) => setNotification('queue_id', next)}
          />
          {credential(ctx, 'notifications.api_token', 'Queue API token', {
            hint: 'A Cloudflare API token with Queues read and write access.',
          })}
        </>
      ) : null}
    </Section>
  )
}

function BucketEditor({
  host,
  name,
  bucket,
  buckets,
  choice,
  nameDraft,
  nameError,
  errors,
  narrow,
  live,
  referencesSupported,
  test,
  onBack,
  onNameDraft,
  onRename,
  onChoice,
  onChange,
  onField,
  onRemove,
  onTest,
}: {
  host: Host
  name: string
  bucket: JsonObject
  buckets: JsonObject
  choice: ProviderChoice
  nameDraft: string
  nameError?: string
  errors: ConfigFormProps['errors']
  narrow: boolean
  live?: LiveBucket
  referencesSupported: boolean | null
  test: TestState | undefined
  onBack: () => void
  onNameDraft: (value: string) => void
  onRename: () => void
  onChoice: (choice: ProviderChoice) => void
  onChange: (next: JsonObject) => void
  onField: (field: string, value: JsonValue | undefined) => void
  onRemove: () => void
  onTest: () => void
}) {
  const provider = providerOf(bucket)
  const display = bucketDisplay(bucket)
  const ctx: CredentialContext = { host, name, bucket, buckets, errors, referencesSupported, onChange, onField }
  const setOptional = (field: string, raw: string) => onChange(withField(bucket, field, raw === '' ? undefined : raw))
  const preset = presetById(detectPreset(asString(bucket.endpoint_url)))
  const compatible = choice === 's3-compatible'
  const choiceInfo = PROVIDER_CHOICES.find((c) => c.value === choice)

  const accessKeys = (idPlaceholder: string, required: boolean) => (
    <>
      {credential(ctx, 'access_key_id', 'Access key ID', {
        mode: 'identifier',
        placeholder: idPlaceholder,
        hint: required
          ? undefined
          : 'Leave both keys empty to use the engine host’s credentials (environment, profile or instance role).',
      })}
      {credential(ctx, 'secret_access_key', 'Secret access key')}
    </>
  )

  return (
    <section className="storage-cfg-editor" data-section={`buckets/${name}`} tabIndex={-1}>
      <EditorHeader
        icon={<BucketIcon name={display.icon} color={display.color} />}
        title={name}
        description={`${providerLabel(choice)} bucket exposed to workers as bucket:${name}.`}
        narrow={narrow}
        onBack={onBack}
        actions={
          <Button variant="ghost" size="sm" onClick={onRemove} aria-label={`Remove ${name}`}>
            <Trash2 /> remove
          </Button>
        }
      />
      <div className="storage-cfg-editor-scroll">
        {live?.error ? (
          <div className="storage-cfg-section">
            <StatusPanel
              variant="alert"
              headline="As saved, the worker cannot build this bucket"
              detail={[stageLabel(live.error_stage), live.error].filter(Boolean).join(' · ')}
            />
          </div>
        ) : null}

        <Section title="Bucket identity" description="The worker-facing name and provider determine request routing.">
          <div className="storage-cfg-field-grid">
            <Field
              label={<label htmlFor={`storage-cfg-${name}-name`}>Worker bucket name</label>}
              hint="Lowercase letters, numbers, underscores, and hyphens; maximum 63 characters."
              error={nameError ?? fieldError(errors, 'buckets', name)}
            >
              <Input
                id={`storage-cfg-${name}-name`}
                data-field={`buckets/${name}`}
                name={`storage-cfg-${name}-name`}
                value={nameDraft}
                spellCheck={false}
                autoComplete="off"
                onChange={onNameDraft}
                onBlur={onRename}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') event.currentTarget.blur()
                }}
              />
            </Field>
            <SelectField<ProviderChoice>
              id={`storage-cfg-${name}-provider`}
              path={`buckets/${name}/provider`}
              label="Provider"
              value={choice}
              options={PROVIDER_CHOICES.map((c) => ({ value: c.value, label: c.label, description: c.description }))}
              hint={choiceInfo?.description}
              error={fieldError(errors, 'buckets', name, 'provider')}
              onChange={onChoice}
            />
          </div>
          <TextField
            id={`storage-cfg-${name}-underlying`}
            path={`buckets/${name}/bucket`}
            label="Provider bucket name"
            value={asString(bucket.bucket)}
            placeholder={name}
            hint={
              provider === 'local'
                ? 'Optional override of its folder under the data directory’s buckets/. Empty uses the worker bucket name.'
                : 'Optional override. Empty uses the worker bucket name.'
            }
            error={fieldError(errors, 'buckets', name, 'bucket')}
            onChange={(next) => setOptional('bucket', next)}
          />
        </Section>

        <Section
          title="Appearance"
          description="How apps draw this bucket: the iii app and this console use the same icon and colour."
        >
          <Field label="Icon">
            <IconPicker
              value={display.icon}
              color={display.color}
              label={`Icon of ${name}`}
              onChange={(icon) => onChange(withDisplay(bucket, 'icon', icon))}
            />
          </Field>
          <Field label="Colour" error={fieldError(errors, 'buckets', name, 'display', 'color')}>
            <ColorPicker
              value={asString(asObject(bucket.display).color) || undefined}
              label={`Colour of ${name}`}
              onChange={(color) => onChange(withDisplay(bucket, 'color', color))}
            />
          </Field>
        </Section>

        {provider === 'local' ? (
          <Section
            title="Native local bucket"
            description="This bucket uses the filesystem and HTTP settings from local storage."
            chip="local"
          >
            <div className="storage-cfg-info-note">
              Direct upload and download URLs are available only when the local HTTP transfer server is enabled in the
              runtime section.
            </div>
          </Section>
        ) : null}

        {provider === 's3' ? (
          <>
            <Section
              title="Connection"
              description={
                compatible
                  ? 'An S3-compatible service: its endpoint, region and addressing.'
                  : 'AWS: the bucket’s region. AWS defaults apply when static credentials are omitted.'
              }
              chip={compatible ? 's3-compatible' : 's3'}
            >
              {compatible ? (
                <SelectField<PresetId>
                  id={`storage-cfg-${name}-preset`}
                  path={`buckets/${name}/endpoint_preset`}
                  label="Service"
                  value={preset.id}
                  options={S3_PRESETS.map((p) => ({ value: p.id, label: p.label }))}
                  hint="Fills in the endpoint, region and addressing; every field stays editable."
                  onChange={(id) => onChange(applyPreset(bucket, id))}
                />
              ) : null}
              <div className="storage-cfg-field-grid">
                <TextField
                  id={`storage-cfg-${name}-region`}
                  path={`buckets/${name}/region`}
                  label="Region"
                  value={asString(bucket.region)}
                  placeholder={compatible ? preset.region : 'us-east-1'}
                  hint={compatible ? preset.regionHint : undefined}
                  error={fieldError(errors, 'buckets', name, 'region')}
                  onChange={(next) => onChange(compatible ? withRegion(bucket, next) : { ...bucket, region: next })}
                />
                {compatible ? (
                  <TextField
                    id={`storage-cfg-${name}-endpoint`}
                    path={`buckets/${name}/endpoint_url`}
                    label="Endpoint URL"
                    value={asString(bucket.endpoint_url)}
                    placeholder={
                      preset.endpoint ? preset.endpoint.replaceAll('{region}', preset.region) : 'https://s3.example.com'
                    }
                    hint="Must be reachable from the engine host. Download links point here too."
                    error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                    onChange={(next) => setOptional('endpoint_url', next)}
                  />
                ) : null}
              </div>
              {compatible ? (
                <CheckField
                  id={`storage-cfg-${name}-path-style`}
                  path={`buckets/${name}/force_path_style`}
                  label="Force path-style addressing"
                  hint="Usually required by self-hosted services (MinIO, Ceph, SeaweedFS); hosted ones accept virtual-hosted addressing."
                  checked={bucket.force_path_style === true}
                  onChange={(checked) => {
                    const next = { ...bucket }
                    if (checked) next.force_path_style = true
                    else delete next.force_path_style
                    onChange(next)
                  }}
                />
              ) : null}
            </Section>
            <Section
              title="Credentials"
              description="Stored in the secrets worker; the configuration keeps a reference."
            >
              <div className="storage-cfg-field-grid">{accessKeys(compatible ? 'Access key' : 'AKIA…', false)}</div>
              {credential(ctx, 'session_token', 'Session token', {
                hint: 'Only for temporary credentials (STS).',
              })}
            </Section>
          </>
        ) : null}

        {provider === 'r2' ? (
          <>
            <Section
              title="Connection"
              description="The standard endpoint is derived from the Cloudflare account ID."
              chip="r2"
            >
              {credential(ctx, 'account_id', 'Account ID', {
                mode: 'identifier',
                preferText: true,
                placeholder: '0123456789abcdef0123456789abcdef',
                hint: 'Shown in the Cloudflare dashboard; it is part of the bucket’s endpoint, not a secret.',
              })}
              <TextField
                id={`storage-cfg-${name}-endpoint`}
                path={`buckets/${name}/endpoint_url`}
                label="Custom endpoint URL"
                value={asString(bucket.endpoint_url)}
                placeholder="Optional R2-compatible endpoint"
                error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                onChange={(next) => setOptional('endpoint_url', next)}
              />
            </Section>
            <Section title="Credentials" description="An R2 API token’s S3 keys, stored in the secrets worker.">
              <div className="storage-cfg-field-grid">{accessKeys('R2 access key ID', true)}</div>
            </Section>
          </>
        ) : null}

        {provider === 'gcs' ? (
          <>
            <Section
              title="Connection"
              description="The engine host’s Application Default Credentials apply when no key is set."
              chip="gcs"
            >
              <TextField
                id={`storage-cfg-${name}-endpoint`}
                path={`buckets/${name}/endpoint_url`}
                label="Custom endpoint URL"
                value={asString(bucket.endpoint_url)}
                placeholder="http://fake-gcs:4443"
                hint="Download links cannot be signed for a custom endpoint; files are then read through the worker."
                error={fieldError(errors, 'buckets', name, 'endpoint_url')}
                onChange={(next) => setOptional('endpoint_url', next)}
              />
            </Section>
            <Section
              title="Credentials"
              description="A service-account key, pasted into the secrets worker or read from a file on the engine host."
            >
              {credential(ctx, 'credentials_json', 'Service-account key', {
                mode: 'json',
                hint: 'The JSON key file’s contents. A key also lets the worker sign download links.',
              })}
              <TextField
                id={`storage-cfg-${name}-credentials-file`}
                path={`buckets/${name}/credentials_file`}
                label="Credentials file"
                value={asString(bucket.credentials_file)}
                placeholder="/etc/iii/gcs-service-account.json"
                hint="A path on the engine host. Set either this or the key above."
                error={fieldError(errors, 'buckets', name, 'credentials_file')}
                onChange={(next) => setOptional('credentials_file', next)}
              />
            </Section>
          </>
        ) : null}

        <Section
          title="Test"
          description="Builds this draft in the worker and lists one object at the root; nothing is saved."
        >
          <TestBucketRow state={test} blocker={testBlocker(bucketProblems(bucket))} onTest={onTest} />
        </Section>

        {provider !== 'local' ? <NotificationFields ctx={ctx} provider={provider} /> : null}
      </div>
    </section>
  )
}

export type StorageConfigFormProps = ConfigFormProps & { host: Host }

export function StorageConfigForm(props: StorageConfigFormProps) {
  const { host } = props
  const value = asObject(props.value)
  const providers = asObject(value.providers)
  const local = asObject(providers.local)
  const buckets = asObject(value.buckets)
  const names = Object.keys(buckets).sort((left, right) => left.localeCompare(right))
  const { ref: rootRef, narrow } = useContainerNarrow({ below: CONFIG_NARROW_BELOW })
  const [selection, setSelection] = useState<Selection>({ kind: 'local' })
  const [narrowPane, setNarrowPane] = useState<'nav' | 'editor'>('nav')
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({})
  const [nameErrors, setNameErrors] = useState<Record<string, string>>({})
  // "S3-compatible" picked while the endpoint is still empty: the entry alone
  // reads as plain S3, so the choice is remembered per bucket.
  const [compatible, setCompatible] = useState<Record<string, boolean>>({})
  const [tests, setTests] = useState<Record<string, TestState>>({})
  const testTokens = useRef<Record<string, number>>(Object.create(null))
  const domRef = useRef<HTMLDivElement | null>(null)
  const valueRef = useRef(value)
  valueRef.current = value
  const focusKey = props.focusField?.join('/') ?? ''
  const { confirm, dialog } = useConfirm()
  const { live, refresh: refreshLive } = useLiveBuckets(host)
  const referencesSupported = supportsSecretReferences(props.schema)

  const setRoot = useCallback(
    (node: HTMLDivElement | null) => {
      rootRef(node)
      domRef.current = node
    },
    [rootRef],
  )

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
    const target =
      domRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`) ??
      domRef.current.querySelector<HTMLElement>(
        `[data-section="${CSS.escape(focusKey.split('/').slice(0, 2).join('/'))}"]`,
      )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [focusKey, selection.kind, selection.kind === 'bucket' ? selection.name : 'local'])

  // Every edit starts from the draft as it is now: a secret stored after an
  // await must not undo what was typed meanwhile.
  const currentBuckets = () => asObject(valueRef.current.buckets)
  const commitBuckets = (nextBuckets: JsonObject) => {
    const next = { ...valueRef.current, buckets: nextBuckets }
    valueRef.current = next
    props.onChange(next)
  }

  // A result is dropped on any edit of its bucket — a stale "Connected" next
  // to a changed key would be a lie. The token discards a test that finishes
  // after such an edit.
  const clearTest = (name: string) => {
    testTokens.current[name] = (testTokens.current[name] ?? 0) + 1
    setTests((current) => {
      if (!(name in current)) return current
      const next = { ...current }
      delete next[name]
      return next
    })
  }

  const setBucket = (name: string, next: JsonObject) => {
    clearTest(name)
    commitBuckets({ ...currentBuckets(), [name]: next })
  }

  // Applied to the entry as it is now; a bucket renamed or removed while a
  // secret was being stored is left alone rather than brought back.
  const setBucketField = (name: string, field: string, next: JsonValue | undefined) => {
    const current = currentBuckets()[name]
    if (!isObject(current)) return
    setBucket(name, withField(current, field, next))
  }

  const addBucket = () => {
    const name = nextBucketName(buckets)
    commitBuckets({ ...buckets, [name]: newBucketEntry('local') })
    setNameDrafts((current) => ({ ...current, [name]: name }))
    choose({ kind: 'bucket', name })
  }

  const renameBucket = (from: string) => {
    const raw = nameDrafts[from] ?? from
    const nextName = raw.trim()
    let error = bucketNameError(nextName) ?? ''
    if (!error && nextName !== from && buckets[nextName] !== undefined) {
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
    clearTest(from)
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
    setCompatible((current) => {
      const next = { ...current, [nextName]: current[from] === true }
      delete next[from]
      return next
    })
    setSelection({ kind: 'bucket', name: nextName })
  }

  const removeBucket = async (name: string) => {
    const ok = await confirm({
      title: `Remove bucket ${name} from the storage configuration?`,
      description: 'Its objects and its secrets stay where they are.',
      confirmLabel: 'Remove',
      tone: 'danger',
    })
    if (!ok) return
    clearTest(name)
    const next = { ...currentBuckets() }
    delete next[name]
    commitBuckets(next)
    setSelection({ kind: 'local' })
    setNarrowPane(narrow ? 'nav' : 'editor')
  }

  const choiceOf = (name: string, bucket: JsonObject): ProviderChoice => {
    const derived = providerChoiceOf(bucket)
    return derived === 's3' && compatible[name] ? 's3-compatible' : derived
  }

  const pickChoice = (name: string, next: ProviderChoice) => {
    const bucket = asObject(currentBuckets()[name])
    setCompatible((current) => ({ ...current, [name]: next === 's3-compatible' }))
    setBucket(name, switchProvider(bucket, next))
  }

  const runTest = async (name: string) => {
    const bucket = asObject(currentBuckets()[name])
    const token = (testTokens.current[name] ?? 0) + 1
    testTokens.current[name] = token
    setTests((current) => ({ ...current, [name]: { status: 'testing' } }))
    let result: TestState
    try {
      const raw = await host.iii.trigger<unknown>('storage::testBucket', testBucketPayload(name, bucket), {
        timeoutMs: 40_000,
      })
      const outcome = parseTestOutcome(raw)
      result = outcome
        ? {
            status: 'done',
            ok: outcome.ok,
            text: testOutcomeText(outcome),
            hint: testOutcomeHint(outcome, underlyingBucket(name, bucket)),
          }
        : { status: 'done', ok: false, text: 'Unexpected answer from storage::testBucket' }
    } catch (cause) {
      result = isMissingFunction(cause)
        ? { status: 'unsupported' }
        : { status: 'done', ok: false, text: errorText(cause) }
    }
    if (testTokens.current[name] !== token) return // superseded by an edit
    setTests((current) => ({ ...current, [name]: result }))
    refreshLive()
  }

  const showNav = !narrow || narrowPane === 'nav'
  const showEditor = !narrow || narrowPane === 'editor'
  const selectedBucket = selection.kind === 'bucket' ? asObject(buckets[selection.name]) : null

  return (
    <div className={`storage-cfg${narrow ? ' narrow' : ''}`} ref={setRoot}>
      {dialog}
      <div className="storage-cfg-workbench">
        {showNav ? (
          <ConfigNav
            buckets={buckets}
            live={live}
            selection={selection}
            localHttpEnabled={isObject(local.http)}
            onSelect={choose}
            onAdd={addBucket}
          />
        ) : null}
        {showEditor && selection.kind === 'local' ? (
          <LocalEditor
            value={value}
            errors={props.errors}
            narrow={narrow}
            onBack={() => setNarrowPane('nav')}
            onChange={props.onChange}
          />
        ) : null}
        {showEditor && selection.kind === 'bucket' && selectedBucket ? (
          <BucketEditor
            key={selection.name}
            host={host}
            name={selection.name}
            bucket={selectedBucket}
            buckets={buckets}
            choice={choiceOf(selection.name, selectedBucket)}
            nameDraft={nameDrafts[selection.name] ?? selection.name}
            nameError={nameErrors[selection.name]}
            errors={props.errors}
            narrow={narrow}
            live={live[selection.name]}
            referencesSupported={referencesSupported}
            test={tests[selection.name]}
            onBack={() => setNarrowPane('nav')}
            onNameDraft={(next) => setNameDrafts((current) => ({ ...current, [selection.name]: next }))}
            onRename={() => renameBucket(selection.name)}
            onChoice={(next) => pickChoice(selection.name, next)}
            onChange={(next) => setBucket(selection.name, next)}
            onField={(field, next) => setBucketField(selection.name, field, next)}
            onRemove={() => void removeBucket(selection.name)}
            onTest={() => void runTest(selection.name)}
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
