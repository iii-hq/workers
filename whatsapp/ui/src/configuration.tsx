import { Badge, Button, Input, RawValueInput, SettingsField, SettingsList, SettingsSection, StatusPanel, Switch, type ConfigFormProps, type Host, type JsonValue } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { ChevronDown } from 'lucide-react'
import { useEffect, useId, useRef, type ReactNode } from 'react'
import { CLOUD_DEFAULTS, configNumber, DEFAULTS, isObject, isOpaqueCloudField, isOpaqueField, withCloudField, withCloudValue, withConfigField, withDatabaseField, type CloudConfigKey, type ConfigKey, type ConfigStatus } from './model'

const FIELDS: { key: ConfigKey; label: string; description: string; min?: number; max?: number }[] = [
  { key: 'data_dir', label: 'Data directory', description: 'Private local session metadata and legacy backups. Keys and messages use the database worker. Changing this path does not move existing data.' },
  { key: 'max_events', label: 'Activity events per session', description: 'Recent session activity kept in memory, separate from the message archive.', min: 1, max: 100000 },
  { key: 'max_inbox_messages', label: 'Stored messages per account', description: 'Limit per native session or Cloud API number. Lowering this removes the oldest stored messages.', min: 1, max: 100000 },
  { key: 'max_sessions', label: 'Session limit', description: 'Limits new sessions; lowering it keeps existing sessions.', min: 1, max: 1024 },
  { key: 'max_media_bytes', label: 'Media limit in bytes', description: 'Maximum size for future media operations.', min: 1, max: 67108864 },
  { key: 'operation_timeout_secs', label: 'Operation timeout', description: 'Seconds allowed for future operations.', min: 1, max: 600 },
  { key: 'shutdown_timeout_secs', label: 'Shutdown timeout', description: 'Seconds allowed when stopping a session.', min: 1, max: 120 },
]
const CLOUD_FIELDS: { key: CloudConfigKey; label: string; description: string; environment: string; secret?: boolean }[] = [
  { key: 'enabled', label: 'Enable Cloud API', description: 'Use the official WhatsApp Business Platform. Linked phone sessions are configured separately.', environment: 'WHATSAPP_CLOUD_ENABLED' },
  { key: 'business_account_id', label: 'Business account ID', description: 'The WhatsApp Business Account (WABA) that owns your number.', environment: 'WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID' },
  { key: 'phone_number_id', label: 'Phone number ID', description: 'The Meta ID of your business phone number, separate from its phone digits.', environment: 'WHATSAPP_CLOUD_PHONE_NUMBER_ID' },
  { key: 'access_token', label: 'Access token', description: 'A token authorized for this WhatsApp Business Account.', environment: 'WHATSAPP_CLOUD_ACCESS_TOKEN', secret: true },
  { key: 'app_secret', label: 'App secret', description: 'Verifies signatures on incoming Meta webhooks.', environment: 'WHATSAPP_META_APP_SECRET', secret: true },
  { key: 'verify_token', label: 'Webhook verify token', description: 'A value you choose and also enter when configuring the webhook in Meta.', environment: 'WHATSAPP_WEBHOOK_VERIFY_TOKEN', secret: true },
  { key: 'api_version', label: 'Graph API version', description: 'Version used for requests to Meta. Default: v26.0.', environment: 'WHATSAPP_CLOUD_API_VERSION' },
]

export function RuntimeStatus({ host, quietWhenApplied = false }: { host: Host; quietWhenApplied?: boolean }) {
  const instance = useId()
  const status = useWorkerLive<ConfigStatus>({ iii: host.iii, triggers: [], pollMs: 10000,
    handlerId: `iii::whatsapp-ui::configuration::${instance}`, fetch: () => host.iii.trigger('whatsapp::config-status') })
  if (status.error) return <StatusPanel variant="warn" role="alert" headline="Active configuration unavailable" detail={status.error} action={<Button variant="ghost" onClick={status.refresh}>Try again</Button>} />
  if (!status.data) return null
  const current = status.data
  let outcome: ReactNode = null
  if (current.last_outcome === 'rejected') outcome = <StatusPanel variant="alert" role="alert" headline="Latest configuration rejected" detail={`${current.last_error ?? 'The previous configuration remains active.'} Rejected reloads: ${current.rejected_reloads}.`} action={<Button variant="ghost" onClick={status.refresh}>Check again</Button>} />
  else if (current.last_outcome === 'restart_required') outcome = <StatusPanel variant="warn" role="status" headline="Some changes need a worker restart" detail={`Pending: ${current.restart_required_fields.map((key) => FIELDS.find((field) => field.key === key)?.label ?? (key === 'database' ? 'Database worker' : key)).join(', ')}. Other changes are active. Current local data: ${current.active.data_dir}; activity limit: ${current.active.max_events}.`} />
  else if (current.source === 'local') outcome = <StatusPanel variant="warn" role="status" headline="Local configuration is active" detail="This worker uses --local-config. Saved ADE changes apply after starting it with central configuration." action={<Button variant="ghost" onClick={status.refresh}>Check again</Button>} />
  else if (!quietWhenApplied) outcome = <p className="wa-ui-note" role="status">Active configuration: <span className="wa-ui-mono">{current.id}</span>. Session data: <span className="wa-ui-mono">{current.active.data_dir}</span>.</p>
  const overrides = Object.entries(current.environment_overrides ?? {})
  return <>{outcome}{overrides.length > 0 && <StatusPanel variant="info" role="status" headline="Cloud API uses environment overrides" detail={<>
    <p>Saved changes do not affect these fields while their environment variables are set. Update the worker environment and restart the worker to change them.</p>
    <details><summary>View overridden fields ({overrides.length})</summary><ul>{overrides.map(([field, variable]) => <li key={field}>{CLOUD_FIELDS.find((item) => `cloud.${item.key}` === field)?.label ?? field}: <code>{variable}</code></li>)}</ul></details>
  </>} />}</>
}

function OpaqueField({ value, label, onChange, onReplace, control, secret = false }: {
  value: JsonValue; label: string; onChange(value: string): void; onReplace(): void
  secret?: boolean
  control: Parameters<import('@iii-dev/console-ui').SettingsFieldProps['renderControl']>[0]
}) {
  return typeof value === 'string'
    ? <RawValueInput {...control} label={label} kind={value.includes('${') ? 'environment' : 'custom'} value={value}
      type={secret ? 'password' : 'text'} autoComplete={secret ? 'new-password' : 'off'} inputClassName="wa-ui-mono" replacementLabel="default value" onChange={onChange} onUseLiteral={onReplace} />
    : <div className="wa-ui-stack"><span className="wa-ui-note">Custom value preserved.</span><Button {...control} variant="ghost" onClick={onReplace}>Use default value</Button></div>
}

export function WhatsAppConfigForm({ host, value, onChange, schema, errors, focusField }: ConfigFormProps & { host: Host }) {
  const root = useRef<HTMLDivElement>(null)
  const advanced = useRef<HTMLDetailsElement>(null)
  const handled = useRef('')
  const focusKey = focusField?.map(String).join('.') ?? ''
  useEffect(() => {
    if (!focusKey) { handled.current = ''; return }
    if (handled.current === focusKey) return
    const target = root.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`)
      ?? (focusKey.startsWith('cloud.') ? root.current?.querySelector<HTMLElement>('[data-field="cloud"]') : null)
      ?? root.current?.querySelector<HTMLElement>('[data-field="configuration"]')
    if (target) {
      const disclosure = target.closest('details')
      if (disclosure) disclosure.open = true
      target.focus(); target.scrollIntoView({ block: 'center' }); handled.current = focusKey
    }
  }, [focusKey, value])
  useEffect(() => {
    if ([...(errors?.keys() ?? [])].some((path) => path === '/cloud/api_version' || path.startsWith('/database') || FIELDS.some((field) => path === `/${field.key}`)) && advanced.current) advanced.current.open = true
  }, [errors])

  const properties = isObject(schema?.properties) ? schema.properties : null
  const cloudProperty = isObject(properties?.cloud) ? properties.cloud : {}
  const cloudDefinition = typeof cloudProperty.$ref === 'string' ? cloudProperty.$ref.split('/').pop() : undefined
  const definitions = isObject(schema?.$defs) ? schema.$defs : isObject(schema?.definitions) ? schema.definitions : {}
  const cloudSchema = cloudDefinition && isObject(definitions[cloudDefinition]) ? definitions[cloudDefinition] : cloudProperty
  const cloudProperties = isObject(cloudSchema.properties) ? cloudSchema.properties : null
  const database = isObject(value) ? value.database : undefined
  const showDatabase = !properties || 'database' in properties
  const cloud = isObject(value) ? value.cloud : undefined
  const showCloud = !properties || 'cloud' in properties
  const cloudField = (item: typeof CLOUD_FIELDS[number]) => {
    if (cloudProperties && !(item.key in cloudProperties)) return null
    const current = isObject(cloud) ? cloud[item.key] : undefined
    const opaque = isOpaqueCloudField(item.key, current)
    return <SettingsField key={item.key} field={`cloud.${item.key}`} label={item.label} description={item.description}
      error={errors?.get(`/cloud/${item.key}`)} controlSize={item.key === 'enabled' && !opaque ? 'fit' : opaque ? 'full' : 'default'}
      layout={item.key === 'enabled' && !opaque ? 'inline' : 'auto'}
      renderControl={(control) => opaque
        ? <OpaqueField control={control} label={item.label} value={current!} secret={item.secret} onChange={(next) => onChange(withCloudField(value, item.key, next))} onReplace={() => onChange(withCloudField(value, item.key, undefined))} />
        : item.key === 'enabled'
          ? <Switch {...control} checked={current === true} onChange={(event) => onChange(withCloudField(value, item.key, event.target.checked))} />
          : <Input {...control} className="wa-ui-mono" type={item.secret ? 'password' : 'text'} value={typeof current === 'string' ? current : ''}
            placeholder={item.key === 'api_version' ? CLOUD_DEFAULTS.api_version : undefined} autoComplete={item.secret ? 'new-password' : 'off'} spellCheck={false}
            onChange={(next) => onChange(withCloudField(value, item.key, next || undefined))} />} />
  }
  const field = (item: typeof FIELDS[number]) => {
    if (properties && !(item.key in properties)) return null
    const current = isObject(value) ? value[item.key] : undefined
    const property = properties?.[item.key]
    const spec = isObject(property) ? property : {}
    const fallback = spec.default ?? DEFAULTS[item.key]
    const shown = current ?? fallback
    return <SettingsField key={item.key} field={item.key} label={item.label} error={errors?.get(`/${item.key}`)}
      description={`${item.description} Default: ${String(fallback)}${item.key === 'max_media_bytes' && typeof shown === 'number' ? ` (${formatBytes(shown)} in this draft)` : ''}.`}
      meta={item.key === 'data_dir' || item.key === 'max_events' ? <Badge>Restart required</Badge> : undefined}
      controlSize={item.key === 'data_dir' || isOpaqueField(item.key, current) ? 'full' : 'compact'}
      layout={item.key === 'data_dir' ? 'stacked' : 'auto'}
      renderControl={(control) => isOpaqueField(item.key, current)
        ? <OpaqueField control={control} label={item.label} value={current!} onChange={(next) => onChange(withConfigField(value, item.key, next))} onReplace={() => onChange(withConfigField(value, item.key, undefined))} />
        : <Input {...control} value={current === undefined ? '' : String(current)} placeholder={String(fallback)}
          type={item.key === 'data_dir' ? 'text' : 'number'} inputMode={item.key === 'data_dir' ? 'text' : 'numeric'}
          min={typeof spec.minimum === 'number' ? spec.minimum : item.min} max={typeof spec.maximum === 'number' ? spec.maximum : item.max} step={1}
          className="wa-ui-mono"
          onChange={(next) => onChange(withConfigField(value, item.key, item.key === 'data_dir' ? next || undefined : configNumber(next)))} />}
    />
  }
  const rootError = errors?.get('') ?? errors?.get('/')
  const visibleFields = new Set(FIELDS.filter((item) => !properties || item.key in properties).map((item) => `/${item.key}`))
  if (showDatabase) { visibleFields.add('/database'); if (database === undefined || isObject(database)) { visibleFields.add('/database/name'); visibleFields.add('/database/namespace') } }
  if (showCloud) {
    visibleFields.add('/cloud')
    if (cloud === undefined || isObject(cloud)) for (const item of CLOUD_FIELDS) if (!cloudProperties || item.key in cloudProperties) visibleFields.add(`/cloud/${item.key}`)
  }
  const unmappedErrors = [...(errors?.entries() ?? [])].filter(([path]) => path && path !== '/' && !visibleFields.has(path)).map(([path, message]) => `${path}: ${message}`)
  return <div ref={root} className="wa-ui-config">
    <header className="wa-ui-config-intro">
      <h2 className="wa-ui-config-heading">WhatsApp configuration</h2>
      <p className="wa-ui-connection-copy">Meta credentials here; phone pairing in Linked phone.</p>
    </header>
    <RuntimeStatus host={host} />
    {!isObject(value) ? <SettingsList><SettingsField field="configuration" label="WhatsApp configuration" description="This entire configuration is a custom value. Replace it only when you are ready to use individual settings." error={rootError} controlSize="full" renderControl={(control) => <OpaqueField control={control} value={value} label="WhatsApp configuration" onChange={onChange} onReplace={() => onChange({})} />} /></SettingsList> : <>
        {showCloud && <>
          <SettingsSection title="Meta Cloud API" description="Use your WhatsApp Business Platform account.">
            {cloud !== undefined && !isObject(cloud) && <SettingsList><SettingsField field="cloud" label="Cloud API configuration" error={errors?.get('/cloud')} description="This custom value is preserved until you choose to replace it with individual fields." controlSize="full" renderControl={(control) => <OpaqueField control={control} value={cloud} label="Cloud API configuration" onChange={(next) => onChange(withCloudValue(value, next))} onReplace={() => onChange(withCloudValue(value, {}))} />} /></SettingsList>}
            {(cloud === undefined || isObject(cloud)) && errors?.get('/cloud') && <StatusPanel variant="alert" headline="Review your Cloud API configuration" detail={errors.get('/cloud')} />}
          </SettingsSection>
          {(cloud === undefined || isObject(cloud)) && <>
            <SettingsSection title="Account" description={<a href="https://developers.facebook.com/documentation/business-messaging/whatsapp/get-started" target="_blank" rel="noopener noreferrer">Meta setup guide</a>}>
              <SettingsList>{CLOUD_FIELDS.slice(0, 4).map(cloudField)}</SettingsList>
              <details className="wa-ui-config-help wa-ui-config-environment"><summary><span>Find account credentials</span><ChevronDown size={16} aria-hidden="true" /></summary><p className="wa-ui-note">Find the WABA and phone number IDs in WhatsApp Manager or your Meta app’s WhatsApp setup. Get a test access token there; create a system user token in Meta Business Settings for ongoing use.</p></details>
            </SettingsSection>
            <SettingsSection title="Webhook credentials" description={<a href="https://developers.facebook.com/docs/graph-api/webhooks/getting-started" target="_blank" rel="noopener noreferrer">Meta webhook guide</a>}>
              <SettingsList>{CLOUD_FIELDS.slice(4, 6).map(cloudField)}</SettingsList>
              <details className="wa-ui-config-help wa-ui-config-environment"><summary><span>Find webhook credentials</span><ChevronDown size={16} aria-hidden="true" /></summary><p className="wa-ui-note">Copy the app secret from App Settings &gt; Basic in your Meta app. Choose your own verify token, then enter the same value here and in Meta’s webhook configuration.</p></details>
            </SettingsSection>
          </>}
          <details className="wa-ui-config-environment"><summary><span>Configure with environment variables</span><ChevronDown size={16} aria-hidden="true" /></summary><p className="wa-ui-note">Environment values override saved fields. Restart the worker after changing them.</p><dl>{CLOUD_FIELDS.filter((item) => !cloudProperties || item.key in cloudProperties).map((item) => <div key={item.key}><dt>{item.label}</dt><dd><code>{item.environment}</code></dd></div>)}</dl></details>
        </>}
        <section className="wa-ui-config-phone"><h3 className="wa-ui-title">Linked phone</h3><p className="wa-ui-connection-copy">Create a session in Linked phone, then open Connection to pair with a QR code or phone number. No Meta credentials are needed.</p></section>
      <details ref={advanced} className="wa-ui-config-advanced"><summary><span>Advanced settings</span><ChevronDown size={16} aria-hidden="true" /></summary><p className="wa-ui-note">Cloud API request version, plus storage, account limits, and timeouts shared by both connection methods.</p><div className="wa-ui-config-grid">
        {showCloud && (cloud === undefined || isObject(cloud)) && (!cloudProperties || 'api_version' in cloudProperties) && <SettingsSection title="Cloud API"><SettingsList>{CLOUD_FIELDS.slice(6).map(cloudField)}</SettingsList></SettingsSection>}
        {showDatabase && <SettingsSection title="Database worker" description="Phone credentials and message archives use the database worker. Database changes require a restart."><SettingsList>
          {database !== undefined && !isObject(database)
            ? <SettingsField field="database" label="Database worker" error={errors?.get('/database')} controlSize="full" renderControl={(control) => <OpaqueField control={control} value={database} label="Database worker" onChange={(next) => onChange({ ...value, database: next })} onReplace={() => onChange({ ...value, database: {} })} />} />
            : (['name', 'namespace'] as const).map((key) => {
              const current = isObject(database) ? database[key] : undefined
              const label = key === 'name' ? 'Database name' : 'Database namespace'
              const fallback = key === 'name' ? 'whatsapp' : ''
              const opaque = current !== undefined && (typeof current !== 'string' || current.includes('${'))
              return <SettingsField key={key} field={`database.${key}`} label={label} meta={<Badge>Restart required</Badge>}
                description={key === 'name' ? 'Named pool configured in the database worker. Default: whatsapp.' : 'Leave empty to use this worker namespace.'}
                error={errors?.get(`/database/${key}`)} controlSize="full" renderControl={(control) => opaque
                  ? <OpaqueField control={control} value={current!} label={label} onChange={(next) => onChange(withDatabaseField(value, key, next))} onReplace={() => onChange(withDatabaseField(value, key, undefined))} />
                  : <Input {...control} className="wa-ui-mono" value={typeof current === 'string' ? current : ''} placeholder={fallback} onChange={(next) => onChange(withDatabaseField(value, key, next || undefined))} />} />
            })}
        </SettingsList></SettingsSection>}
        <SettingsSection className="wa-ui-config-storage" title="Storage & history"><SettingsList>{FIELDS.slice(0, 3).map(field)}</SettingsList></SettingsSection>
        <SettingsSection className="wa-ui-config-capacity" title="Capacity"><SettingsList>{FIELDS.slice(3, 5).map(field)}</SettingsList></SettingsSection>
        <SettingsSection className="wa-ui-config-timeouts" title="Time limits"><SettingsList>{FIELDS.slice(5).map(field)}</SettingsList></SettingsSection>
      </div></details>
      {rootError && <StatusPanel variant="alert" headline="Review your configuration" detail={rootError} />}
      {unmappedErrors.length > 0 && <StatusPanel variant="alert" headline="Other values need your attention" detail={unmappedErrors.join('\n')} />}
      <p className="wa-ui-note wa-ui-config-footer">Saved changes apply live unless a field is controlled by the environment or marked Restart required. Leave a field empty to use its default.</p>
    </>}
  </div>
}
