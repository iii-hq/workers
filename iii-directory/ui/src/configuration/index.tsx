/**
 * Custom configuration form for the `iii-directory` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * configuration surface with a worker-owned interface.
 *
 * The form edits the working draft via `onChange`; dirty tracking,
 * save/reset, and error mapping stay host-owned (the console's SaveBar
 * below the form drives `configuration::set`). Mirrors SkillsConfig
 * (workers/iii-directory/src/config.rs): skills_folder,
 * local_skills_folder, agent profile/skill roots, registry_url, timeouts,
 * filtering, auto-download, function search, search hints, and registry search.
 */

import {
  Chip,
  type ConfigFormProps,
  Input,
  type JsonValue,
  Select,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import {
  booleanWithDefault,
  FUNCTION_SEARCH_MODE_OPTIONS,
  type FunctionSearchMode,
  functionSearchModeWithDefault,
  semanticModeNeedsModel,
  withFunctionSearchMode,
} from './model'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

/** Fields from `SkillsConfig::topology()` — changing them needs a worker
 * restart (the hot-reload path refuses them). */
const TOPOLOGY_FIELDS = new Set([
  'skills_folder',
  'local_skills_folder',
  'agents_folder',
  'agents_skills_folder',
  'auto_download',
  'function_search_model_path',
])

const INLINE_ERROR_POINTERS = new Set([
  '/skills_folder',
  '/local_skills_folder',
  '/agents_folder',
  '/global_agents_folder',
  '/agents_skills_folder',
  '/global_agents_skills_folder',
  '/registry_url',
  '/download_timeout_ms',
  '/registry_cache_ttl_ms',
  '/filter_unregistered',
  '/auto_download',
  '/inject_hint',
  '/hint_min_workers',
  '/registry_search',
  '/function_search_mode',
  '/function_search_model_path',
])

export function DirectoryConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const unassociatedErrors = [...(props.errors?.entries() ?? [])].filter(
    ([pointer]) => !INLINE_ERROR_POINTERS.has(pointer),
  )

  const commit = (next: JsonObject) => props.onChange(next)

  const setString = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else next[field] = raw
    commit(next)
  }

  const setNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else if (!Number.isNaN(Number(raw))) next[field] = Number(raw)
    commit(next)
  }

  const setBool = (field: string, checked: boolean) => {
    commit({ ...value, [field]: checked })
  }

  const setFunctionSearchMode = (mode: FunctionSearchMode) => {
    commit(withFunctionSearchMode(value, mode))
  }

  // Each explicit form owns the field markup, so it also honors deep-link
  // focus requests from global Settings.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(field)}"]`)
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    focusable?.focus()
  }, [props.focusField])

  return (
    <div className="dir-ui-form" ref={rootRef}>
      <SettingsSection
        title="Content locations"
        description="Choose the project and user-level folders scanned for skills and reusable agent profiles."
      >
        <SettingsList>
          <TextField
            field="skills_folder"
            label="Skills folder"
            placeholder="skills"
            hint="Global root scanned by every read and used for downloads. Paths may be absolute, start with ~/, or be relative to III_COMPOSE_DIR."
            value={asString(value.skills_folder)}
            onChange={setString}
            errors={props.errors}
          />
          <TextField
            field="local_skills_folder"
            label="Local skills folder"
            placeholder="skills/iii"
            hint="Project-scoped overrides relative to III_COMPOSE_DIR. A namespace here takes precedence over the global folder."
            value={asString(value.local_skills_folder)}
            onChange={setString}
            errors={props.errors}
          />
          <TextField
            field="agents_folder"
            label="Agent profiles folder"
            placeholder="agents"
            hint="Read-write root for reusable agent profile Markdown files."
            value={asString(value.agents_folder)}
            onChange={setString}
            errors={props.errors}
          />
          <TextField
            field="global_agents_folder"
            label="Global agent profiles folder"
            placeholder="~/.iii/agents"
            hint="User-level profiles shared by every project. A project profile with the same ID takes precedence."
            value={asString(value.global_agents_folder)}
            onChange={setString}
            errors={props.errors}
          />
          <TextField
            field="agents_skills_folder"
            label="External agent skills folder"
            placeholder=".agents/skills"
            hint="Project-level, read-only skills scanned as <skill>/SKILL.md relative to III_COMPOSE_DIR."
            value={asString(value.agents_skills_folder)}
            onChange={setString}
            errors={props.errors}
          />
          <TextField
            field="global_agents_skills_folder"
            label="Global external agent skills folder"
            placeholder="~/.agents/skills"
            hint="User-level, read-only skills shared by every project. Project and iii skill roots take precedence."
            value={asString(value.global_agents_skills_folder)}
            onChange={setString}
            errors={props.errors}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Worker registry"
        description="Configure downloads, registry requests, and cached responses."
      >
        <SettingsList>
          <TextField
            field="registry_url"
            label="Registry URL"
            placeholder="https://api.workers.iii.dev"
            hint="Registry used by download and proxy functions."
            value={asString(value.registry_url)}
            onChange={setString}
            errors={props.errors}
          />
          <NumberField
            field="download_timeout_ms"
            label="Download timeout (ms)"
            placeholder="60000"
            hint="Applies to each HTTP request or git clone and to registry proxy requests."
            value={value.download_timeout_ms}
            onChange={setNumber}
            errors={props.errors}
          />
          <NumberField
            field="registry_cache_ttl_ms"
            label="Registry cache TTL (ms)"
            placeholder="60000"
            hint="How long registry list and info responses stay cached in process. Set to 0 to disable caching."
            value={value.registry_cache_ttl_ms}
            onChange={setNumber}
            errors={props.errors}
          />
          <CheckField
            field="registry_search"
            label="Include installable workers"
            hint="Search the public registry and include matches from verified authors under installable results."
            checked={booleanWithDefault(value.registry_search, true)}
            onChange={setBool}
            errors={props.errors}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Skill availability"
        description="Control which installed content is visible and how new workers acquire their skills."
      >
        <SettingsList>
          <CheckField
            field="filter_unregistered"
            label="Hide skills from uninstalled workers"
            hint="Show only namespaces that match a registered worker. Turn this off to include everything on disk."
            checked={booleanWithDefault(value.filter_unregistered, true)}
            onChange={setBool}
            errors={props.errors}
          />
          <CheckField
            field="auto_download"
            label="Download skills when workers are added"
            hint="Subscribe to worker additions and reconcile missing skill bundles at startup."
            checked={booleanWithDefault(value.auto_download, true)}
            onChange={setBool}
            errors={props.errors}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Function search"
        description="Choose how installed function contracts are ranked. Mode changes apply without restarting iii-directory."
      >
        <SettingsList>
          <SearchModeField
            value={functionSearchModeWithDefault(value.function_search_mode)}
            modelPath={value.function_search_model_path}
            onChange={setFunctionSearchMode}
            errors={props.errors}
          />
          <TextField
            field="function_search_model_path"
            label="Semantic model directory"
            placeholder="not set — e.g. ~/.cache/iii/all-MiniLM-L6-v2-<revision>"
            hint="Existing directory containing the local semantic model. The runtime never downloads a model, and changing this path requires a worker restart."
            value={asString(value.function_search_model_path)}
            onChange={setString}
            errors={props.errors}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Search guidance"
        description="Decide when the model receives a hint about directory search."
      >
        <SettingsList>
          <CheckField
            field="inject_hint"
            label="Inject the search hint"
            hint="Bind the directory::pre-generate hook. When off, the model finds search_functions only through normal discovery."
            checked={booleanWithDefault(value.inject_hint, false)}
            onChange={setBool}
            errors={props.errors}
          />
          <NumberField
            field="hint_min_workers"
            label="Minimum worker surface"
            placeholder="2"
            hint="Inject the hint only when the session exposes at least this many distinct non-engine workers. Set to 0 to hint on every surface."
            value={value.hint_min_workers}
            onChange={setNumber}
            errors={props.errors}
          />
        </SettingsList>
      </SettingsSection>

      {props.errors && props.errors.size > 0 ? (
        <div className="dir-ui-form-errors" role="alert">
          <div>
            {props.errors.size === 1
              ? 'There is 1 configuration error. Review the highlighted setting.'
              : `There are ${props.errors.size} configuration errors. Review the highlighted settings.`}
          </div>
          {unassociatedErrors.map(([pointer, message]) => (
            <div key={pointer || message}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function describedBy(...ids: Array<string | undefined>): string | undefined {
  const value = ids.filter(Boolean).join(' ')
  return value || undefined
}

function FieldLabel({ field, htmlFor, label }: { field: string; htmlFor: string; label: React.ReactNode }) {
  return (
    <label className="dir-ui-config-label" htmlFor={htmlFor}>
      {label}
      {TOPOLOGY_FIELDS.has(field) ? <Chip tone="warning">Restart required</Chip> : null}
    </label>
  )
}

function fieldError(field: string, errors: ConfigFormProps['errors']) {
  return errors?.get(`/${field}`)
}

function fieldPresentation(field: string, hint: string | undefined, errors: ConfigFormProps['errors']) {
  const id = `dir-cfg-${field}`
  const descriptionId = hint ? `${id}-description` : undefined
  const error = fieldError(field, errors)
  const errorId = error ? `${id}-error` : undefined
  return {
    id,
    description: hint ? <span id={descriptionId}>{hint}</span> : undefined,
    meta: error ? (
      <span className="dir-ui-config-error" id={errorId}>
        {error}
      </span>
    ) : undefined,
    describedBy: describedBy(descriptionId, errorId),
    invalid: Boolean(error),
  }
}

function TextField({
  field,
  label,
  placeholder,
  hint,
  value,
  onChange,
  errors,
}: {
  field: string
  label: string
  placeholder: string
  hint?: string
  value: string
  onChange: (field: string, raw: string) => void
  errors?: ConfigFormProps['errors']
}) {
  const presentation = fieldPresentation(field, hint, errors)
  return (
    <SettingsRow
      data-field={field}
      label={<FieldLabel field={field} htmlFor={presentation.id} label={label} />}
      description={presentation.description}
      meta={presentation.meta}
      control={
        <Input
          id={presentation.id}
          name={field}
          className="dir-ui-config-control"
          value={value}
          placeholder={placeholder}
          spellCheck={false}
          autoComplete="off"
          aria-label={label}
          aria-invalid={presentation.invalid || undefined}
          aria-describedby={presentation.describedBy}
          onChange={(next) => onChange(field, next)}
        />
      }
    />
  )
}

function NumberField({
  field,
  label,
  placeholder,
  hint,
  value,
  onChange,
  errors,
}: {
  field: string
  label: string
  placeholder: string
  hint?: string
  value: JsonValue | undefined
  onChange: (field: string, raw: string) => void
  errors?: ConfigFormProps['errors']
}) {
  const presentation = fieldPresentation(field, hint, errors)
  return (
    <SettingsRow
      data-field={field}
      label={<FieldLabel field={field} htmlFor={presentation.id} label={label} />}
      description={presentation.description}
      meta={presentation.meta}
      control={
        <Input
          id={presentation.id}
          name={field}
          className="dir-ui-config-control dir-ui-config-number"
          type="number"
          min={0}
          inputMode="numeric"
          value={typeof value === 'number' ? String(value) : ''}
          placeholder={placeholder}
          aria-label={label}
          aria-invalid={presentation.invalid || undefined}
          aria-describedby={presentation.describedBy}
          onChange={(next) => onChange(field, next)}
        />
      }
    />
  )
}

function SearchModeField({
  value,
  modelPath,
  onChange,
  errors,
}: {
  value: FunctionSearchMode
  modelPath: JsonValue | undefined
  onChange: (mode: FunctionSearchMode) => void
  errors?: ConfigFormProps['errors']
}) {
  const field = 'function_search_mode'
  const hint =
    'Lexical returns BM25 rankings. Shadow computes semantic rankings without changing results. Hybrid fuses BM25 with the local semantic model.'
  const presentation = fieldPresentation(field, hint, errors)
  const needsModel = semanticModeNeedsModel(value, modelPath)
  const noticeId = needsModel ? `${presentation.id}-model-notice` : undefined
  const modeLabel = FUNCTION_SEARCH_MODE_OPTIONS.find((option) => option.value === value)?.label
  const notice = needsModel ? (
    <span className="dir-ui-config-notice" id={noticeId}>
      <Chip tone="warning">Model required</Chip>
      <span>
        {modeLabel} requires a local semantic model. Set its directory below and restart iii-directory; searches use
        lexical fallback until then.
      </span>
    </span>
  ) : null

  return (
    <SettingsRow
      data-field={field}
      label={<FieldLabel field={field} htmlFor={presentation.id} label="Function search mode" />}
      description={presentation.description}
      meta={
        presentation.meta || notice ? (
          <>
            {presentation.meta}
            {notice}
          </>
        ) : undefined
      }
      control={
        <Select<FunctionSearchMode>
          id={presentation.id}
          name={field}
          data-field={field}
          className="dir-ui-config-control dir-ui-config-select"
          value={value}
          options={[...FUNCTION_SEARCH_MODE_OPTIONS]}
          aria-label="Function search mode"
          aria-invalid={presentation.invalid || undefined}
          aria-describedby={describedBy(presentation.describedBy, noticeId)}
          sheetTitle="Function search mode"
          sheetDescription="Choose the ranking lane used by directory::search_functions."
          onChange={onChange}
        />
      }
    />
  )
}

function CheckField({
  field,
  label,
  hint,
  checked,
  onChange,
  errors,
}: {
  field: string
  label: string
  hint?: string
  checked: boolean
  onChange: (field: string, checked: boolean) => void
  errors?: ConfigFormProps['errors']
}) {
  const presentation = fieldPresentation(field, hint, errors)
  return (
    <SettingsRow
      data-field={field}
      label={<FieldLabel field={field} htmlFor={presentation.id} label={label} />}
      description={presentation.description}
      meta={presentation.meta}
      layout="inline"
      control={
        <Switch
          id={presentation.id}
          checked={checked}
          aria-label={label}
          aria-invalid={presentation.invalid || undefined}
          aria-describedby={presentation.describedBy}
          onChange={(event) => onChange(field, event.currentTarget.checked)}
        />
      }
    />
  )
}
