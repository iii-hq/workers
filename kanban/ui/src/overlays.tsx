import {
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  ListItem,
  RawValueInput,
  Select,
  Selector,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  iii,
  type ConfigFormProps,
  type Host,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Plus, Trash2 } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import {
  PLAN_CHANGED_EVENT,
  api,
  priorityLabel,
  statusLabel,
  type AgentProfile,
  type Column,
  type ConfigInfo,
  type Ticket,
} from './shared'
import { NO_FACETS, laneName, resolveDefaults, type Facets, type NewTicketDefaults, type Plan } from './subtask-rules'
import { ParentSelector, ProjectSelector, ReleaseSelector } from './subtasks'

type CreateTicketDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  host: Host
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  defaultStatus: string
  /** Opened from a swimlane cell: the column and the project, release or parent the new ticket starts with. */
  defaults?: NewTicketDefaults
  onCreated: (ticket: Ticket) => void
}

export function CreateTicketDialog({
  host,
  open,
  onOpenChange,
  columns,
  priorities,
  agents,
  defaultStatus,
  defaults,
  onCreated,
}: CreateTicketDialogProps) {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [status, setStatus] = useState(defaults?.status ?? defaultStatus)
  const [priority, setPriority] = useState(priorities[0] ?? 'medium')
  const [assignee, setAssignee] = useState<string | undefined>(undefined)
  const [facets, setFacets] = useState<Facets>(NO_FACETS)
  // What the swimlane put in the facets, for the hint under them; null until the board is read.
  const [seeded, setSeeded] = useState<Facets>(NO_FACETS)
  // The swimlane's own name, kept from the opening so the closing dialog doesn't change its sentence.
  const [laneLabel, setLaneLabel] = useState<string | null>(null)
  const [plan, setPlan] = useState<Plan | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const hintId = useId()
  // The board hands over fresh objects on every reload: only opening the dialog reads them, so a reload while
  // it is open never wipes what was typed.
  const defaultsRef = useRef(defaults)
  defaultsRef.current = defaults
  const defaultStatusRef = useRef(defaultStatus)
  defaultStatusRef.current = defaultStatus
  const prioritiesRef = useRef(priorities)
  prioritiesRef.current = priorities

  useEffect(() => {
    if (!open) return
    setTitle('')
    setDescription('')
    setStatus(defaultsRef.current?.status ?? defaultStatusRef.current)
    // The worker files a ticket without a priority under the first configured one.
    setPriority(prioritiesRef.current[0] ?? 'medium')
    setAssignee(undefined)
    setFacets(NO_FACETS)
    setSeeded(NO_FACETS)
    setLaneLabel(defaultsRef.current?.lane ?? null)
    setPlan(null)
    setError(null)
    setBusy(false)
  }, [open])

  // Projects, releases and the tickets a new one can hang from: read when the dialog opens and again when a
  // project or release is written elsewhere. Only the first read seeds the facets; later ones keep the user's
  // picks and drop what is gone or closed to new tickets.
  useEffect(() => {
    if (!open) return
    let cancelled = false
    let seeding = true
    let latest = 0
    const read = () => {
      const request = ++latest
      void api
        .board(host.iii)
        .then((board) => {
          if (cancelled || request !== latest) return
          const next = {
            projects: board.projects ?? [],
            releases: board.releases ?? [],
            tickets: board.columns.flatMap((column) => column.tickets),
          }
          setPlan(next)
          if (seeding) {
            seeding = false
            const seed = resolveDefaults(next, defaultsRef.current)
            setFacets(seed)
            setSeeded(seed)
          } else {
            setFacets((now) => resolveDefaults(next, now))
          }
        })
        // Without the board the facets stay empty; the ticket can still be created.
        .catch(() => {
          if (!cancelled && request === latest && seeding) setPlan({ projects: [], releases: [], tickets: [] })
        })
    }
    read()
    window.addEventListener(PLAN_CHANGED_EVENT, read)
    return () => {
      cancelled = true
      window.removeEventListener(PLAN_CHANGED_EVENT, read)
    }
  }, [open, host])

  const setFacet = (kind: keyof Facets, next: string | null) => setFacets((now) => ({ ...now, [kind]: next }))
  const prefilled = (kind: keyof Facets) => Boolean(seeded[kind]) && seeded[kind] === facets[kind]
  const hint = (kind: keyof Facets) =>
    prefilled(kind) ? (
      <span className="kanban-field__hint" id={`${hintId}-${kind}`}>
        Prefilled from the swimlane
      </span>
    ) : undefined
  const lane = plan ? laneName(plan, seeded, laneLabel) : laneLabel

  const submit = async () => {
    const trimmed = title.trim()
    if (!trimmed || busy) return
    setBusy(true)
    setError(null)
    try {
      const ticket = await api.create(host.iii, {
        title: trimmed,
        description,
        status,
        priority,
        assignee: assignee ?? null,
        ...(facets.project ? { project: facets.project } : {}),
        ...(facets.release ? { release: facets.release } : {}),
        ...(facets.parent ? { parent: facets.parent } : {}),
        actor: 'user',
      })
      onCreated(ticket)
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="kanban-dialog">
        <DialogTitle>New ticket</DialogTitle>
        <DialogDescription>
          {lane
            ? `Opened from the ${lane} swimlane, in the ${statusLabel(columns, status)} column.`
            : 'It is added to the board, and the ticket opens beside it.'}
        </DialogDescription>
        <div
          className="kanban-dialog__form"
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey && event.target instanceof HTMLInputElement) {
              event.preventDefault()
              void submit()
            }
          }}
        >
          <label className="kanban-field">
            <span className="kanban-field__label">Title</span>
            <Input autoFocus onChange={setTitle} placeholder="What needs doing?" value={title} />
          </label>
          <label className="kanban-field">
            <span className="kanban-field__label">Description</span>
            <textarea
              className="kanban-field__textarea"
              onChange={(event) => setDescription(event.target.value)}
              placeholder="Markdown supported"
              rows={4}
              value={description}
            />
          </label>
          <div className="kanban-dialog__grid">
            <label className="kanban-field">
              <span className="kanban-field__label">Status</span>
              <Select
                onChange={setStatus}
                options={columns.map((column) => ({ value: column.id, label: column.label }))}
                value={status}
              />
            </label>
            <label className="kanban-field">
              <span className="kanban-field__label">Priority</span>
              <Select
                onChange={setPriority}
                options={priorities.map((value) => ({ value, label: priorityLabel(value) }))}
                value={priority}
              />
            </label>
            <div className="kanban-field">
              <span className="kanban-field__label">Assignee</span>
              <Selector
                allowEmpty
                aria-label="Assignee"
                emptyLabel="Unassigned"
                onChange={setAssignee}
                onClear={() => setAssignee(undefined)}
                options={agents.map((agent) => ({
                  value: agent.id,
                  label: `${agent.logo ? `${agent.logo} ` : ''}${agent.name}`,
                  description: agent.description ?? undefined,
                }))}
                placeholder="Unassigned"
                value={assignee}
              />
            </div>
          </div>
          <div className="kanban-dialog__grid">
            <div className="kanban-field">
              <span className="kanban-field__label">Project</span>
              <ProjectSelector
                aria-describedby={prefilled('project') ? `${hintId}-project` : undefined}
                onChange={(next) => setFacet('project', next)}
                projects={plan?.projects ?? null}
                value={facets.project}
              />
              {hint('project')}
            </div>
            <div className="kanban-field">
              <span className="kanban-field__label">Release</span>
              <ReleaseSelector
                aria-describedby={prefilled('release') ? `${hintId}-release` : undefined}
                onChange={(next) => setFacet('release', next)}
                releases={plan?.releases ?? null}
                value={facets.release}
              />
              {hint('release')}
            </div>
            <div className="kanban-field">
              <span className="kanban-field__label">Parent</span>
              <ParentSelector
                aria-describedby={prefilled('parent') ? `${hintId}-parent` : undefined}
                current={null}
                onChange={(next) => setFacet('parent', next)}
                selfId={null}
                tickets={plan?.tickets ?? null}
                value={facets.parent}
              />
              {hint('parent')}
            </div>
          </div>
          {error ? <p className="kanban-field__error">{error}</p> : null}
        </div>
        <div className="kanban-dialog__foot">
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button disabled={busy || title.trim().length === 0} onClick={() => void submit()} variant="primary">
            {busy ? 'Creating…' : 'Create ticket'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

type ColumnDraft = { id: string; label: string }

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? { ...(value as Record<string, unknown>) }
    : {}
}

function readColumns(record: Record<string, unknown>): ColumnDraft[] {
  const raw = Array.isArray(record.columns) ? record.columns : []
  const columns: ColumnDraft[] = []
  for (const entry of raw) {
    const item = asRecord(entry)
    const id = typeof item.id === 'string' ? item.id : ''
    if (!id) continue
    columns.push({ id, label: typeof item.label === 'string' ? item.label : id })
  }
  return columns
}

function readPriorities(record: Record<string, unknown>): string[] {
  const raw = Array.isArray(record.priorities) ? record.priorities : []
  return raw.filter((entry): entry is string => typeof entry === 'string')
}

export function KanbanConfigForm({ value, onChange, errors, focusField }: ConfigFormProps) {
  const record = asRecord(value)
  const columns = readColumns(record)
  const priorities = readPriorities(record)
  const dataPath = typeof record.data_path === 'string' ? record.data_path : ''
  const agentsPath = typeof record.agents_path === 'string' ? record.agents_path : ''
  const idPrefix = typeof record.id_prefix === 'string' ? record.id_prefix : ''
  const defaultStatus = typeof record.default_status === 'string' ? record.default_status : ''
  // Unset, or a column that is gone: the worker repairs it to the last column, so that is what the form shows.
  const storedDone = typeof record.done_status === 'string' ? record.done_status : ''
  const doneStatus = columns.some((column) => column.id === storedDone) ? storedDone : (columns[columns.length - 1]?.id ?? '')
  const [info, setInfo] = useState<ConfigInfo | null>(null)
  const [activeColumn, setActiveColumn] = useState<number | null>(null)
  const consumed = useRef<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void api
      .config(iii)
      .then((result) => {
        if (!cancelled) setInfo(result)
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [])

  const focusKey = focusField && focusField.length > 0 ? focusField.map(String).join('.') : ''
  useEffect(() => {
    if (!focusKey || consumed.current === focusKey) return
    consumed.current = focusKey
    if (focusField && focusField[0] === 'columns') {
      const index = Number(focusField[1])
      if (Number.isInteger(index) && index >= 0 && index < columns.length) setActiveColumn(index)
    }
  }, [focusKey, focusField, columns.length])

  const update = (changes: Record<string, unknown>) => onChange({ ...record, ...changes } as never)

  const updateColumn = (index: number, changes: Partial<ColumnDraft>) => {
    const next = columns.map((column, position) =>
      position === index ? { ...column, ...changes } : column,
    )
    // Renaming the done or the default column's id keeps it that column.
    const renamed = changes.id !== undefined ? columns[index]?.id : undefined
    update({
      columns: next,
      ...(renamed !== undefined && renamed === doneStatus ? { done_status: changes.id } : {}),
      ...(renamed !== undefined && renamed === defaultStatus ? { default_status: changes.id } : {}),
    })
  }

  const addColumn = () => {
    let suffix = columns.length + 1
    while (columns.some((column) => column.id === `column_${suffix}`)) suffix += 1
    const next = [...columns, { id: `column_${suffix}`, label: `Column ${suffix}` }]
    // Done is the last column while done_status is unset (or gone): write it down, or the new column takes it.
    update({ columns: next, ...(doneStatus && storedDone !== doneStatus ? { done_status: doneStatus } : {}) })
    setActiveColumn(next.length - 1)
  }

  const removeColumn = (index: number) => {
    const next = columns.filter((_, position) => position !== index)
    update({ columns: next })
    setActiveColumn(null)
  }

  const fieldFocus = (field: string): string | undefined => (focusKey === field ? field : undefined)

  const active = activeColumn !== null ? columns[activeColumn] : undefined

  return (
    <>
      <SettingsSection
        description="Where the board file lives. A relative path resolves from the project root."
        title="Storage"
      >
        <SettingsList>
          <SettingsField
            data-field="data_path"
            description="Folder holding board.json."
            error={errors?.get('/data_path')}
            field={fieldFocus('data_path')}
            id="kanban-data-path"
            label="Board folder"
            renderControl={(controlProps) =>
              dataPath.includes('${') ? (
                <RawValueInput
                  {...controlProps}
                  kind="environment"
                  label="Board folder"
                  onChange={(next) => update({ data_path: next })}
                  onUseLiteral={() => update({ data_path: info?.data_path_resolved ?? '' })}
                  replacementLabel="Use the resolved path"
                  value={dataPath}
                />
              ) : (
                <Input
                  {...controlProps}
                  onChange={(next) => update({ data_path: next })}
                  placeholder="./data/kanban"
                  value={dataPath}
                />
              )
            }
          />
          <SettingsField
            data-field="id_prefix"
            description="Prefix for the human-readable key, for example KAN -> KAN-1."
            error={errors?.get('/id_prefix')}
            field={fieldFocus('id_prefix')}
            id="kanban-id-prefix"
            label="Ticket key prefix"
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                onChange={(next) => update({ id_prefix: next.toUpperCase().replace(/[^A-Z0-9]/g, '') })}
                placeholder="KAN"
                value={idPrefix}
              />
            )}
          />
          <SettingsField
            data-field="default_status"
            description="Column a new ticket lands in."
            error={errors?.get('/default_status')}
            field={fieldFocus('default_status')}
            id="kanban-default-status"
            label="Default column"
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                onChange={(next) => update({ default_status: next })}
                options={columns.map((column) => ({ value: column.id, label: column.label }))}
                value={defaultStatus}
              />
            )}
          />
          <SettingsField
            data-field="done_status"
            description="Progress, subtask counts and releases count a ticket as done only in this column. In review is not done."
            error={errors?.get('/done_status')}
            field={fieldFocus('done_status')}
            id="kanban-done-status"
            label="Done column"
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                onChange={(next) => update({ done_status: next })}
                options={columns.map((column) => ({ value: column.id, label: column.label }))}
                value={doneStatus}
              />
            )}
          />
          <SettingsField
            data-field="priorities"
            description="Comma-separated, lowest first. Offered on the ticket."
            error={errors?.get('/priorities')}
            field={fieldFocus('priorities')}
            id="kanban-priorities"
            label="Priorities"
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                onChange={(next) =>
                  update({
                    priorities: next
                      .split(',')
                      .map((entry: string) => entry.trim())
                      .filter((entry: string) => entry.length > 0),
                  })
                }
                placeholder="low, medium, high, urgent"
                value={priorities.join(', ')}
              />
            )}
          />
          <SettingsField
            data-field="agents_path"
            description="Folder of agent profiles read when iii-directory is not installed."
            error={errors?.get('/agents_path')}
            field={fieldFocus('agents_path')}
            id="kanban-agents-path"
            label="Agents folder"
            renderControl={(controlProps) => (
              <Input {...controlProps} onChange={(next) => update({ agents_path: next })} placeholder="agents" value={agentsPath} />
            )}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        action={
          <Button onClick={addColumn} size="sm" variant="pill">
            <Plus size={16} />
            Add column
          </Button>
        }
        description="Left to right on the board. The id is what tickets store."
        title="Columns"
      >
        <SettingsDeck
          autoFocusDetail={focusKey === ''}
          backLabel="Columns"
          detail={
            active && activeColumn !== null ? (
              <SettingsList>
                <SettingsField
                  data-field={`columns.${activeColumn}.id`}
                  description="Stored on every ticket in this column."
                  field={focusKey === `columns.${activeColumn}.id` ? focusKey : undefined}
                  id="kanban-column-id"
                  label="Column id"
                  renderControl={(controlProps) => (
                    <Input
                      {...controlProps}
                      onChange={(next) =>
                        updateColumn(activeColumn, { id: next.toLowerCase().replace(/[^a-z0-9_]/g, '_') })
                      }
                      value={active.id}
                    />
                  )}
                />
                <SettingsField
                  data-field={`columns.${activeColumn}.label`}
                  description="Shown on the lane header."
                  field={focusKey === `columns.${activeColumn}.label` ? focusKey : undefined}
                  id="kanban-column-label"
                  label="Column name"
                  renderControl={(controlProps) => (
                    <Input
                      {...controlProps}
                      onChange={(next) => updateColumn(activeColumn, { label: next })}
                      value={active.label}
                    />
                  )}
                />
                <SettingsRow
                  action={
                    <Button
                      disabled={columns.length <= 1}
                      onClick={() => removeColumn(activeColumn)}
                      size="sm"
                      variant="ghost"
                    >
                      <Trash2 size={16} />
                      Remove column
                    </Button>
                  }
                  description="Tickets in it keep this status id."
                  label="Remove"
                  layout="stacked"
                />
              </SettingsList>
            ) : null
          }
          onBack={() => setActiveColumn(null)}
          open={activeColumn !== null}
          overview={
            <SettingsList>
              {columns.length === 0 ? (
                <SettingsRow description="Add one to start the board." label="No columns" />
              ) : null}
              {columns.map((column, index) => (
                <ListItem
                  description={column.id}
                  key={`${column.id}-${index}`}
                  label={column.label}
                  onClick={() => setActiveColumn(index)}
                  trailing={
                    column.id === defaultStatus || column.id === doneStatus ? (
                      <>
                        {column.id === defaultStatus ? <Chip>Default column</Chip> : null}
                        {column.id === doneStatus ? <Chip>Done column</Chip> : null}
                      </>
                    ) : undefined
                  }
                />
              ))}
            </SettingsList>
          }
          title={active?.label ?? 'Column'}
        />
      </SettingsSection>

      <SettingsSection description="Read-only, resolved by the worker." title="Where it is stored">
        <SettingsList>
          <SettingsRow
            description={info?.data_path_resolved ?? 'Resolving…'}
            label="Board folder"
            layout="stacked"
            meta="data_path_resolved"
          />
          <SettingsRow
            description={info?.board_file ?? 'Resolving…'}
            label="Board file"
            layout="stacked"
            meta="board_file"
          />
          <SettingsRow
            description={info?.project_root ?? 'Resolving…'}
            label="Project root"
            layout="stacked"
            meta="project_root"
          />
          <SettingsRow
            description={info?.agents_path_resolved ?? 'Resolving…'}
            label="Agents folder"
            layout="stacked"
            meta="agents_path_resolved"
          />
          <SettingsRow
            description={
              info
                ? `${info.columns.length} columns · ${info.priorities.length} priorities · done is ${statusLabel(info.columns, info.done_status)}`
                : 'Resolving…'
            }
            label="Effective board"
            layout="stacked"
          />
        </SettingsList>
      </SettingsSection>
    </>
  )
}
