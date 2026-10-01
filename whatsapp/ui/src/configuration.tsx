import { Badge, Button, Input, RawValueInput, SettingsField, SettingsList, SettingsSection, StatusPanel, type ConfigFormProps, type Host, type JsonValue } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useEffect, useId, useRef } from 'react'
import { configNumber, DEFAULTS, isObject, isOpaqueField, withConfigField, type ConfigKey, type ConfigStatus } from './model'

const FIELDS: { key: ConfigKey; label: string; description: string; min?: number; max?: number }[] = [
  { key: 'data_dir', label: 'Data directory', description: 'Relative to the worker directory. Changing this path does not move existing data.' },
  { key: 'max_events', label: 'Activity events per session', description: 'Recent session activity kept in memory, separate from the message archive.', min: 1, max: 100000 },
  { key: 'max_inbox_messages', label: 'Stored messages per session', description: 'Captured messages and received history. Lowering this removes the oldest stored messages.', min: 1, max: 100000 },
  { key: 'max_sessions', label: 'Session limit', description: 'Limits new sessions; lowering it keeps existing sessions.', min: 1, max: 1024 },
  { key: 'max_media_bytes', label: 'Media limit in bytes', description: 'Maximum size for future media operations.', min: 1, max: 67108864 },
  { key: 'operation_timeout_secs', label: 'Operation timeout', description: 'Seconds allowed for future operations.', min: 1, max: 600 },
  { key: 'shutdown_timeout_secs', label: 'Shutdown timeout', description: 'Seconds allowed when stopping a session.', min: 1, max: 120 },
]

export function RuntimeStatus({ host, quietWhenApplied = false }: { host: Host; quietWhenApplied?: boolean }) {
  const instance = useId()
  const status = useWorkerLive<ConfigStatus>({ iii: host.iii, triggers: [], pollMs: 10000,
    handlerId: `iii::whatsapp-ui::configuration::${instance}`, fetch: () => host.iii.trigger('whatsapp::config-status') })
  if (status.error) return <StatusPanel variant="warn" headline="Active configuration unavailable" detail={status.error} action={<Button variant="ghost" onClick={status.refresh}>Try again</Button>} />
  if (!status.data) return null
  const current = status.data
  if (current.last_outcome === 'rejected') return <StatusPanel variant="alert" headline="Latest configuration rejected" detail={`${current.last_error ?? 'The previous configuration remains active.'} Rejected reloads: ${current.rejected_reloads}.`} action={<Button variant="ghost" onClick={status.refresh}>Check again</Button>} />
  if (current.last_outcome === 'restart_required') return <StatusPanel variant="warn" headline="Some changes need a worker restart" detail={`Pending: ${current.restart_required_fields.map((key) => FIELDS.find((field) => field.key === key)?.label ?? key).join(', ')}. Other changes are active. Current storage: ${current.active.data_dir}; activity limit: ${current.active.max_events}.`} />
  if (current.source === 'local') return <StatusPanel variant="warn" headline="Local configuration is active" detail="This worker uses --local-config. Saved ADE changes apply after starting it with central configuration." action={<Button variant="ghost" onClick={status.refresh}>Check again</Button>} />
  if (quietWhenApplied) return null
  return <p className="wa-ui-note">Active configuration: <span className="wa-ui-mono">{current.id}</span>. Session data: <span className="wa-ui-mono">{current.active.data_dir}</span>.</p>
}

function OpaqueField({ value, label, onChange, onReplace, control }: {
  value: JsonValue; label: string; onChange(value: string): void; onReplace(): void
  control: Parameters<import('@iii-dev/console-ui').SettingsFieldProps['renderControl']>[0]
}) {
  return typeof value === 'string'
    ? <RawValueInput {...control} label={label} kind={value.includes('${') ? 'environment' : 'custom'} value={value}
      replacementLabel="default value" onChange={onChange} onUseLiteral={onReplace} />
    : <div className="wa-ui-stack"><span className="wa-ui-note">Custom value preserved.</span><Button {...control} variant="ghost" onClick={onReplace}>Use default value</Button></div>
}

export function WhatsAppConfigForm({ host, value, onChange, schema, errors, focusField }: ConfigFormProps & { host: Host }) {
  const root = useRef<HTMLDivElement>(null)
  const handled = useRef('')
  const focusKey = focusField?.map(String).join('.') ?? ''
  useEffect(() => {
    if (!focusKey) { handled.current = ''; return }
    if (handled.current === focusKey) return
    const target = root.current?.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusKey)}"]`)
      ?? root.current?.querySelector<HTMLElement>('[data-field="configuration"]')
    if (target) { target.focus(); target.scrollIntoView({ block: 'center' }); handled.current = focusKey }
  }, [focusKey, value])

  const properties = isObject(schema?.properties) ? schema.properties : null
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
          className={item.key === 'data_dir' ? 'wa-ui-mono' : undefined}
          onChange={(next) => onChange(withConfigField(value, item.key, item.key === 'data_dir' ? next || undefined : configNumber(next)))} />}
    />
  }
  const rootError = errors?.get('') ?? errors?.get('/')
  const visibleFields = new Set(FIELDS.filter((item) => !properties || item.key in properties).map((item) => `/${item.key}`))
  const unmappedErrors = [...(errors?.entries() ?? [])].filter(([path]) => path && path !== '/' && !visibleFields.has(path)).map(([path, message]) => `${path}: ${message}`)
  return <div ref={root} className="wa-ui-config">
    <header className="wa-ui-config-intro">
      <h2 className="wa-ui-config-heading">Storage and limits</h2>
      <p className="wa-ui-connection-copy">Choose what to store and set operating limits.</p>
    </header>
    <RuntimeStatus host={host} />
    {!isObject(value) ? <SettingsList><SettingsField field="configuration" label="WhatsApp configuration" description="This entire configuration is a custom value. Replace it only when you are ready to use individual settings." error={rootError} controlSize="full" renderControl={(control) => <OpaqueField control={control} value={value} label="WhatsApp configuration" onChange={onChange} onReplace={() => onChange({})} />} /></SettingsList> : <>
      <div className="wa-ui-config-grid">
        <SettingsSection className="wa-ui-config-storage" title="Storage & history" description="Session credentials and message archives stay in the data directory."><SettingsList>{FIELDS.slice(0, 3).map(field)}</SettingsList></SettingsSection>
        <SettingsSection className="wa-ui-config-capacity" title="Capacity"><SettingsList>{FIELDS.slice(3, 5).map(field)}</SettingsList></SettingsSection>
        <SettingsSection className="wa-ui-config-timeouts" title="Time limits"><SettingsList>{FIELDS.slice(5).map(field)}</SettingsList></SettingsSection>
      </div>
      {rootError && <StatusPanel variant="alert" headline="Review your configuration" detail={rootError} />}
      {unmappedErrors.length > 0 && <StatusPanel variant="alert" headline="Other values need your attention" detail={unmappedErrors.join('\n')} />}
      <p className="wa-ui-note wa-ui-config-footer">Saved changes apply live unless marked Restart required. Leave a field empty to use its default.</p>
    </>}
  </div>
}
