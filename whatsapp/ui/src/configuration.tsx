import { Button, Input, RawValueInput, SettingsField, SettingsList, SettingsSection, StatusPanel, type ConfigFormProps, type Host, type JsonValue } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useEffect, useId, useRef } from 'react'
import { configNumber, DEFAULTS, isObject, isOpaqueField, withConfigField, type ConfigKey, type ConfigStatus } from './model'

const FIELDS: { key: ConfigKey; label: string; description: string; min?: number; max?: number }[] = [
  { key: 'data_dir', label: 'Diretório de dados', description: 'Credenciais SQLite e metadados locais de cada sessão. Relativo ao diretório do worker. Alterar exige reiniciar; os dados existentes não são movidos.' },
  { key: 'max_events', label: 'Eventos retidos por sessão', description: 'Histórico em memória usado pela atividade. Alterar exige reiniciar o worker.', min: 1, max: 100000 },
  { key: 'max_sessions', label: 'Limite de sessões', description: 'Aplica ao criar novas sessões; reduzir não remove as existentes.', min: 1, max: 1024 },
  { key: 'max_media_bytes', label: 'Limite de mídia em bytes', description: 'Aplica às próximas operações de mídia.', min: 1, max: 67108864 },
  { key: 'operation_timeout_secs', label: 'Tempo limite das operações', description: 'Em segundos. Aplica às próximas chamadas.', min: 1, max: 600 },
  { key: 'shutdown_timeout_secs', label: 'Tempo limite de parada', description: 'Em segundos. Aplica às próximas paradas de sessão.', min: 1, max: 120 },
]

export function RuntimeStatus({ host, quietWhenApplied = false }: { host: Host; quietWhenApplied?: boolean }) {
  const instance = useId()
  const status = useWorkerLive<ConfigStatus>({ iii: host.iii, triggers: [], pollMs: 10000,
    handlerId: `iii::whatsapp-ui::configuration::${instance}`, fetch: () => host.iii.trigger('whatsapp::config-status') })
  if (status.error) return <StatusPanel variant="warn" headline="Não foi possível consultar a configuração ativa" detail={status.error} action={<Button variant="ghost" onClick={status.refresh}>Tentar novamente</Button>} />
  if (!status.data) return null
  const current = status.data
  if (current.last_outcome === 'rejected') return <StatusPanel variant="alert" headline="A última configuração foi rejeitada" detail={`${current.last_error ?? 'O worker manteve a configuração anterior.'} Recargas rejeitadas: ${current.rejected_reloads}.`} action={<Button variant="ghost" onClick={status.refresh}>Verificar novamente</Button>} />
  if (current.last_outcome === 'restart_required') return <StatusPanel variant="warn" headline="Reinicie o worker para aplicar todas as alterações" detail={`Campos pendentes: ${current.restart_required_fields.join(', ')}. As demais alterações já foram aplicadas. Diretório em uso: ${current.active.data_dir}. Eventos em uso: ${current.active.max_events}.`} />
  if (current.source === 'local') return <StatusPanel variant="warn" headline="Configuração local em uso" detail="O worker iniciou com --local-config. Alterações salvas no ADE não serão aplicadas até iniciar o worker com a configuração central." action={<Button variant="ghost" onClick={status.refresh}>Verificar novamente</Button>} />
  if (quietWhenApplied) return null
  return <p className="wa-ui-note">Configuração central ativa: <span className="wa-ui-mono">{current.id}</span>. Credenciais em <span className="wa-ui-mono">{current.active.data_dir}</span>.</p>
}

function OpaqueField({ value, label, onChange, onReplace, control }: {
  value: JsonValue; label: string; onChange(value: string): void; onReplace(): void
  control: Parameters<import('@iii-dev/console-ui').SettingsFieldProps['renderControl']>[0]
}) {
  return typeof value === 'string'
    ? <RawValueInput {...control} label={label} kind={value.includes('${') ? 'environment' : 'custom'} value={value}
      replacementLabel="Usar valor padrão" onChange={onChange} onUseLiteral={onReplace} />
    : <div className="wa-ui-stack"><span className="wa-ui-note">Valor personalizado preservado.</span><Button {...control} variant="ghost" onClick={onReplace}>Usar valor padrão</Button></div>
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
      description={`${item.description} Padrão: ${String(fallback)}${item.key === 'max_media_bytes' && typeof shown === 'number' ? ` (${formatBytes(shown)} em uso no rascunho)` : ''}.`}
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
    <RuntimeStatus host={host} />
    {!isObject(value) ? <SettingsList><SettingsField field="configuration" label="Configuração do WhatsApp" description="O valor completo é personalizado. Ele só será substituído por uma ação explícita." error={rootError} controlSize="full" renderControl={(control) => <OpaqueField control={control} value={value} label="Configuração do WhatsApp" onChange={onChange} onReplace={() => onChange({})} />} /></SettingsList> : <>
      <SettingsSection title="Armazenamento e histórico" description="As credenciais ficam no diretório de dados do worker, fora da configuração do projeto. Parear uma conta é uma ação separada na página WhatsApp."><SettingsList>{FIELDS.slice(0, 2).map(field)}</SettingsList></SettingsSection>
      <SettingsSection title="Limites e tempos" description="Alterações válidas são aplicadas sem reiniciar. Deixe um campo vazio para restaurar o padrão."><SettingsList>{FIELDS.slice(2).map(field)}</SettingsList></SettingsSection>
      {rootError && <StatusPanel variant="alert" headline="Revise a configuração" detail={rootError} />}
      {unmappedErrors.length > 0 && <StatusPanel variant="alert" headline="Há outros valores que precisam de revisão" detail={unmappedErrors.join('\n')} />}
    </>}
  </div>
}
