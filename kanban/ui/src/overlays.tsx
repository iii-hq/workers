import {
  Button,
  Chip,
  Input,
  ListItem,
  RawValueInput,
  Select,
  SettingsDeck,
  ShadcnButton,
  ShadcnDialog,
  ShadcnDialogContent,
  ShadcnDialogDescription,
  ShadcnDialogTitle,
  ShadcnSwitch,
  ShadcnTextarea,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  iii,
  type ConfigFormProps,
  type Host,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ChevronRight, Plus, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  PLAN_CHANGED_EVENT,
  api,
  statusLabel,
  type AgentProfile,
  type Column,
  type ConfigInfo,
  type Ticket,
} from './shared'
import { NO_FACETS, laneName, resolveDefaults, type Facets, type NewTicketDefaults, type Plan } from './subtask-rules'
import { AssigneePicker, ParentPicker, PriorityMenu, ProjectPicker, ReleasePicker, StatusMenu } from './ui/properties'

type CreateTicketDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  host: Host
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  defaultStatus: string
  /** For the status glyphs: the board's done and canceled columns. */
  doneStatus?: string
  canceledStatus?: string | null
  /** Opened from a swimlane cell: the column and the project, release or parent the new ticket starts with. */
  defaults?: NewTicketDefaults
  /** `keepOpen`: "Create more" is on — the dialog stays open for the next ticket. */
  onCreated: (ticket: Ticket, options?: { keepOpen?: boolean }) => void
}

export function CreateTicketDialog({
  host,
  open,
  onOpenChange,
  columns,
  priorities,
  agents,
  defaultStatus,
  doneStatus = '',
  canceledStatus = null,
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
  // Linear's "Create more": the dialog stays open, cleared, for the next ticket.
  const [createMore, setCreateMore] = useState(false)
  const titleRef = useRef<HTMLInputElement>(null)
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
      if (createMore) {
        onCreated(ticket, { keepOpen: true })
        setTitle('')
        setDescription('')
        titleRef.current?.focus()
      } else {
        onCreated(ticket)
      }
    } catch (caught) {
      setError(errorMessage(caught))
    } finally {
      setBusy(false)
    }
  }

  const prefilledAny = (['project', 'release', 'parent'] as const).some(prefilled)
  return (
    <ShadcnDialog onOpenChange={onOpenChange} open={open}>
      <ShadcnDialogContent className="kb-dialog kb-new-ticket">
        <header className="kb-new-ticket__top">
          <span className="kb-new-ticket__crumb">
            <span className="kb-new-ticket__crumb-root">Kanban</span>
            {lane ? (
              <>
                <ChevronRight aria-hidden="true" size={16} />
                <span className="kb-new-ticket__crumb-root">{lane}</span>
              </>
            ) : null}
            <ChevronRight aria-hidden="true" size={16} />
            <span>New ticket</span>
          </span>
        </header>
        <ShadcnDialogTitle className="kb-sr-only">New ticket</ShadcnDialogTitle>
        <ShadcnDialogDescription className="kb-sr-only">
          {lane
            ? `Opened from the ${lane} swimlane, in the ${statusLabel(columns, status)} column.`
            : `Added to the ${statusLabel(columns, status)} column; the ticket opens beside the board.`}
        </ShadcnDialogDescription>
        <div
          className="kb-new-ticket__body"
          onKeyDown={(event) => {
            if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
              event.preventDefault()
              void submit()
              return
            }
            if (event.key === 'Enter' && !event.shiftKey && event.target instanceof HTMLInputElement) {
              event.preventDefault()
              void submit()
            }
          }}
        >
          <input
            aria-label="Title"
            autoFocus
            className="kb-new-ticket__title"
            onChange={(event) => setTitle(event.target.value)}
            placeholder="Ticket title"
            ref={titleRef}
            value={title}
          />
          <ShadcnTextarea
            aria-label="Description"
            className="kb-new-ticket__description"
            onChange={(event) => setDescription(event.target.value)}
            placeholder="Add description…"
            rows={4}
            value={description}
          />
        </div>
        <div aria-label="Properties" className="kb-new-ticket__props" role="group">
          <StatusMenu
            canceledStatus={canceledStatus}
            columns={columns}
            doneStatus={doneStatus}
            onChange={setStatus}
            status={status}
          />
          <PriorityMenu onChange={setPriority} priorities={priorities} priority={priority} />
          <AssigneePicker agents={agents} onChange={(next) => setAssignee(next ?? undefined)} value={assignee ?? null} />
          <ProjectPicker onChange={(next) => setFacet('project', next)} projects={plan?.projects ?? null} value={facets.project} />
          <ReleasePicker onChange={(next) => setFacet('release', next)} releases={plan?.releases ?? null} value={facets.release} />
          <ParentPicker
            current={null}
            onChange={(next) => setFacet('parent', next)}
            selfId={null}
            tickets={plan?.tickets ?? null}
            value={facets.parent}
          />
        </div>
        {prefilledAny || error ? (
          <div className="kb-new-ticket__notes">
            {prefilledAny ? <p className="kb-field__hint">Project, release or parent prefilled from the swimlane.</p> : null}
            {error ? (
              <p className="kb-field__error" role="alert">
                {error}
              </p>
            ) : null}
          </div>
        ) : null}
        <footer className="kb-new-ticket__foot">
          <label className="kb-new-ticket__more">
            <ShadcnSwitch checked={createMore} onCheckedChange={setCreateMore} />
            Create more
          </label>
          <span className="kb-new-ticket__spacer" />
          <ShadcnButton onClick={() => onOpenChange(false)} size="sm" type="button" variant="ghost">
            Cancel
          </ShadcnButton>
          <ShadcnButton disabled={busy || title.trim().length === 0} onClick={() => void submit()} size="sm" type="button">
            {busy ? 'Creating…' : 'Create ticket'}
            <span aria-hidden="true" className="kb-new-ticket__kbd">
              ⌘↵
            </span>
          </ShadcnButton>
        </footer>
      </ShadcnDialogContent>
    </ShadcnDialog>
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
  // Read the way the worker repairs them, so the form shows what the board uses. Canceled: a missing key means the
  // `canceled` column, null or empty means none.
  const requestedCanceled =
    record.canceled_status === undefined
      ? 'canceled'
      : typeof record.canceled_status === 'string'
        ? record.canceled_status.trim()
        : ''
  // Done: unset, or a column that is gone, falls to the last column that is not the canceled one.
  const storedDone = typeof record.done_status === 'string' ? record.done_status.trim() : ''
  const doneFallback =
    [...columns].reverse().find((column) => column.id !== requestedCanceled) ?? columns[columns.length - 1]
  const doneStatus = columns.some((column) => column.id === storedDone) ? storedDone : (doneFallback?.id ?? '')
  // A canceled id that names no column, or the done one, means none.
  const canceledStatus =
    requestedCanceled !== doneStatus && columns.some((column) => column.id === requestedCanceled)
      ? requestedCanceled
      : ''
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
    // Renaming the done, canceled or default column's id keeps it that column.
    const renamed = changes.id !== undefined ? columns[index]?.id : undefined
    update({
      columns: next,
      ...(renamed !== undefined && renamed === doneStatus ? { done_status: changes.id } : {}),
      ...(renamed !== undefined && renamed === canceledStatus ? { canceled_status: changes.id } : {}),
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
                // The done column cannot also be the canceled one: the worker would drop the canceled one anyway.
                onChange={(next) =>
                  update({ done_status: next, ...(next === canceledStatus ? { canceled_status: null } : {}) })
                }
                options={columns.map((column) => ({ value: column.id, label: column.label }))}
                value={doneStatus}
              />
            )}
          />
          <SettingsField
            data-field="canceled_status"
            description="Tickets here are canceled: they leave progress and release totals and never hold a release open."
            error={errors?.get('/canceled_status')}
            field={fieldFocus('canceled_status')}
            id="kanban-canceled-status"
            label="Canceled column"
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                allowEmpty
                emptyLabel="None"
                onChange={(next) => update({ canceled_status: next })}
                onClear={() => update({ canceled_status: null })}
                options={columns
                  .filter((column) => column.id !== doneStatus)
                  .map((column) => ({ value: column.id, label: column.label }))}
                placeholder="None"
                value={canceledStatus || undefined}
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
                    column.id === defaultStatus || column.id === doneStatus || column.id === canceledStatus ? (
                      <>
                        {column.id === defaultStatus ? <Chip>Default column</Chip> : null}
                        {column.id === doneStatus ? <Chip>Done column</Chip> : null}
                        {column.id === canceledStatus ? <Chip>Canceled column</Chip> : null}
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
                ? `${info.columns.length} columns · ${info.priorities.length} priorities · done is ${statusLabel(info.columns, info.done_status)}${info.canceled_status ? ` · canceled is ${statusLabel(info.columns, info.canceled_status)}` : ' · no canceled column'}`
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
