import {
  Button,
  type ConfigFormProps,
  Input,
  ListItem,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { Plus, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

type Draft = Record<string, unknown>
type Path = (string | number)[]

const asRecord = (value: unknown): Draft =>
  value && typeof value === 'object' && !Array.isArray(value) ? { ...(value as Draft) } : {}

/** `undefined` deletes the key: the worker's default comes back. */
function patch(target: Draft, key: string, next: unknown): Draft {
  const copy = { ...target }
  if (next === undefined) delete copy[key]
  else copy[key] = next
  return copy
}

const text = (value: unknown) => (typeof value === 'string' ? value : '')
const csv = (value: unknown) =>
  Array.isArray(value) ? value.filter((entry) => typeof entry === 'string').join(', ') : ''
const fromCsv = (value: string) => {
  const entries = value
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
  return entries.length ? entries : undefined
}

export function StoriesConfigForm({ value, onChange, errors, focusField }: ConfigFormProps) {
  const record = asRecord(value)
  const viewport = asRecord(record.viewport)
  const workspaces = (Array.isArray(record.workspaces) ? record.workspaces : []).map(asRecord)
  const [active, setActive] = useState<number | null>(null)
  const opened = useRef('')
  const focused = useRef('')

  // A deep link opens its workspace first, then focuses the exact field once
  // the deck has mounted it; each request is consumed once.
  const focusKey = focusField?.length ? focusField.map(String).join('.') : ''
  // `active` re-runs the lookup after the deck mounts the detail.
  useEffect(() => {
    if (!focusKey) return
    if (opened.current !== focusKey) {
      opened.current = focusKey
      const index = focusField?.[0] === 'workspaces' ? Number(focusField[1]) : Number.NaN
      if (Number.isInteger(index) && index >= 0 && index < workspaces.length) setActive(index)
    }
    if (focused.current === focusKey) return
    const target = document.querySelector<HTMLElement>(`[data-iii-ui="stories"] [data-field="${CSS.escape(focusKey)}"]`)
    if (!target) return
    focused.current = focusKey
    target.scrollIntoView({ block: 'center' })
    target.focus()
  }, [focusKey, active])

  const update = (key: string, next: unknown) => onChange(patch(record, key, next) as never)
  const updateWorkspace = (index: number, key: string, next: unknown) =>
    update(
      'workspaces',
      workspaces.map((entry, position) => (position === index ? patch(entry, key, next) : entry)),
    )
  const addWorkspace = () => {
    let suffix = workspaces.length + 1
    while (workspaces.some((entry) => entry.name === `workspace-${suffix}`)) suffix += 1
    update('workspaces', [...workspaces, { name: `workspace-${suffix}`, path: '.' }])
    setActive(workspaces.length)
  }

  /** One labelled input; `path` is its place in the draft (pointer, deep link and id). */
  const field = (
    path: Path,
    label: string,
    current: string,
    apply: (next: string) => void,
    options: { description?: string; placeholder?: string; numeric?: boolean } = {},
  ) => (
    <SettingsField
      description={options.description}
      error={errors?.get(`/${path.join('/')}`)}
      field={path.join('.')}
      id={`stories-${path.join('-')}`}
      label={label}
      renderControl={(props) => (
        <Input
          {...props}
          inputMode={options.numeric ? 'numeric' : undefined}
          onChange={apply}
          placeholder={options.placeholder}
          value={current}
        />
      )}
    />
  )
  /** Empty means "use the default" (the key goes away); `NaN` is never committed. */
  const numberField = (
    path: Path,
    label: string,
    source: Draft,
    fallback: number,
    apply: (next: number | undefined) => void,
    description?: string,
  ) => {
    const stored = source[String(path[path.length - 1])]
    return field(
      path,
      label,
      typeof stored === 'number' ? String(stored) : '',
      (next) => {
        const parsed = Number(next)
        if (next.trim() === '') apply(undefined)
        else if (Number.isInteger(parsed) && parsed > 0) apply(parsed)
      },
      { description, placeholder: String(fallback), numeric: true },
    )
  }
  const viewportField = (key: string, label: string, fallback: number, description?: string) =>
    numberField(
      ['viewport', key],
      label,
      viewport,
      fallback,
      (next) => update('viewport', patch(viewport, key, next)),
      description,
    )

  const current = active !== null ? workspaces[active] : undefined
  const workspaceField = (
    key: string,
    label: string,
    description: string,
    placeholder?: string,
    parse: (next: string) => unknown = (next) => next,
  ) =>
    active !== null && current
      ? field(
          ['workspaces', active, key],
          label,
          Array.isArray(current[key]) ? csv(current[key]) : text(current[key]),
          (next) => updateWorkspace(active, key, parse(next)),
          { description, placeholder },
        )
      : null

  return (
    <>
      <SettingsSection
        description="Where snapshots live and how headless renders reach the console."
        title="Storage and rendering"
      >
        <SettingsList>
          {field(
            ['data_path'],
            'Data folder',
            text(record.data_path),
            (next) => update('data_path', next || undefined),
            {
              description: 'Folder holding lines, files and renders. A relative path resolves from the project root.',
              placeholder: 'data/stories',
            },
          )}
          {field(
            ['console_url'],
            'Console url',
            text(record.console_url),
            (next) => update('console_url', next || undefined),
            {
              description: 'Origin the browser worker opens for screenshots and trees.',
              placeholder: 'http://127.0.0.1:3113',
            },
          )}
          <SettingsField
            controlSize="fit"
            description="Rebuild the working tree when a story input changes."
            error={errors?.get('/watch')}
            field="watch"
            id="stories-watch"
            label="Watch"
            layout="inline"
            renderControl={(props) => (
              <Switch
                {...props}
                checked={record.watch !== false}
                onChange={(event) => update('watch', event.currentTarget.checked)}
              />
            )}
          />
          {numberField(
            ['keep_lines'],
            'Keep lines',
            record,
            12,
            (next) => update('keep_lines', next),
            'Built ref lines kept per workspace before the oldest are pruned.',
          )}
          {viewportField('width', 'Viewport width', 1024, 'Default viewport for screenshots, in CSS pixels.')}
          {viewportField('height', 'Viewport height', 768)}
          {viewportField('dpr', 'Device scale', 1, 'Device scale factor, 1 to 3.')}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        action={
          <Button onClick={addWorkspace} size="sm" variant="pill">
            <Plus aria-hidden size={16} />
            Add workspace
          </Button>
        }
        description="Repositories whose story files are indexed. Projects inside each are discovered from the files."
        title="Workspaces"
      >
        <SettingsDeck
          autoFocusDetail={focusKey === ''}
          backLabel="Workspaces"
          detail={
            current && active !== null ? (
              <SettingsList>
                {workspaceField('name', 'Name', 'Slug used in function calls and urls.', undefined, (next) =>
                  next.toLowerCase().replace(/[^a-z0-9_-]/g, '-'),
                )}
                {workspaceField('path', 'Path', 'Repository root, relative to the project directory or absolute.', '.')}
                {workspaceField(
                  'base',
                  'Base ref',
                  'Git ref the explorer compares the working tree against. Default HEAD.',
                  'HEAD',
                  (next) => next.trim() || undefined,
                )}
                {workspaceField(
                  'stories',
                  'Story globs',
                  'Comma-separated globs, relative to each project. Default **/*.stories.{tsx,ts,jsx,js}.',
                  'src/**/*.stories.tsx',
                  fromCsv,
                )}
                {workspaceField(
                  'ignore',
                  'Ignore',
                  'Comma-separated folder names or globs skipped while discovering.',
                  'fixtures, legacy/**',
                  fromCsv,
                )}
                <SettingsRow
                  action={
                    <Button
                      disabled={workspaces.length <= 1}
                      onClick={() => {
                        update(
                          'workspaces',
                          workspaces.filter((_, position) => position !== active),
                        )
                        setActive(null)
                      }}
                      size="sm"
                      variant="ghost"
                    >
                      <Trash2 aria-hidden size={16} />
                      Remove workspace
                    </Button>
                  }
                  description="Its snapshots stay on disk until pruned."
                  label="Remove"
                  layout="stacked"
                />
              </SettingsList>
            ) : null
          }
          onBack={() => setActive(null)}
          open={active !== null}
          overview={
            <SettingsList>
              {workspaces.length === 0 ? (
                <SettingsRow description="Add one to start indexing." label="No workspaces" />
              ) : null}
              {workspaces.map((entry, index) => (
                <ListItem
                  description={text(entry.path)}
                  key={`${text(entry.name)}-${index}`}
                  label={text(entry.name)}
                  onClick={() => setActive(index)}
                />
              ))}
            </SettingsList>
          }
          title={text(current?.name) || 'Workspace'}
        />
      </SettingsSection>
    </>
  )
}
