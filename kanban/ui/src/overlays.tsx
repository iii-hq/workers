import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  List,
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
import { useEffect, useRef, useState } from 'react'
import {
  PlusIcon,
  TrashIcon,
  api,
  errorMessage,
  type AgentProfile,
  type Column,
  type ConfigInfo,
  type Ticket,
} from './shared'

type CreateTicketDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  host: Host
  columns: Column[]
  priorities: string[]
  agents: AgentProfile[]
  defaultStatus: string
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
  onCreated,
}: CreateTicketDialogProps) {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [status, setStatus] = useState(defaultStatus)
  const [priority, setPriority] = useState(priorities[0] ?? 'medium')
  const [assignee, setAssignee] = useState<string | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setTitle('')
    setDescription('')
    setStatus(defaultStatus)
    setPriority(priorities[0] ?? 'medium')
    setAssignee(undefined)
    setError(null)
    setBusy(false)
  }, [open, defaultStatus, priorities])

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
          It is added to the board, and the ticket opens beside it.
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
                options={priorities.map((value) => ({ value, label: value }))}
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
    update({ columns: next })
  }

  const addColumn = () => {
    let suffix = columns.length + 1
    while (columns.some((column) => column.id === `column_${suffix}`)) suffix += 1
    const next = [...columns, { id: `column_${suffix}`, label: `Column ${suffix}` }]
    update({ columns: next })
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
            <PlusIcon />
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
                      <TrashIcon />
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
            description={info ? `${info.columns.length} columns · ${info.priorities.length} priorities` : 'Resolving…'}
            label="Effective board"
            layout="stacked"
          />
        </SettingsList>
      </SettingsSection>
    </>
  )
}
