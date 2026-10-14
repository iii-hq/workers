/**
 * New session, new window and rename. Each dialog is mounted only while
 * open, so every opening starts from empty fields; a refused request keeps
 * the dialog and shows the worker's reason under the form.
 */

import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  Selector,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { type FormEvent, type ReactNode, useId, useMemo, useState } from 'react'
import { validateCommand, validateDirectory, validateName } from '../lib/model'

function Field({
  id,
  label,
  hint,
  error,
  children,
}: {
  id: string
  label: string
  hint?: ReactNode
  error?: string | null
  children: ReactNode
}) {
  return (
    <div className={`${uiClasses.field} tmux-dialog-field`}>
      <label className={uiClasses.fieldLabel} htmlFor={id}>
        {label}
      </label>
      {children}
      {error ? (
        <p className={uiClasses.fieldError} id={`${id}-error`} role="alert">
          {error}
        </p>
      ) : hint ? (
        <p className={uiClasses.fieldDescription} id={`${id}-hint`}>
          {hint}
        </p>
      ) : null}
    </div>
  )
}

function DirectoryField({
  id,
  value,
  onChange,
  recent,
  error,
}: {
  id: string
  value: string
  onChange(next: string): void
  recent: readonly string[]
  error: string | null
}) {
  const options = useMemo(() => {
    const seen = new Set<string>()
    const list: { value: string; label: string }[] = []
    for (const dir of [value, ...recent]) {
      if (!dir || seen.has(dir)) continue
      seen.add(dir)
      list.push({ value: dir, label: dir })
    }
    return list
  }, [recent, value])
  return (
    <Field
      id={id}
      label="Directory"
      hint="Pick a recent folder or type an absolute path (~/ works). Empty uses tmux's default."
      error={error}
    >
      <Selector
        id={id}
        aria-label="Directory"
        aria-invalid={error ? true : undefined}
        value={value || undefined}
        options={options}
        onChange={onChange}
        onCreate={(query) => onChange(query.trim())}
        createOptionLabel={(query) => `Use ${query.trim()}`}
        placeholder="Default directory"
        searchPlaceholder="Type or filter a path…"
        emptyMessage="Type a path to use it."
        allowEmpty
        emptyLabel="Default directory"
        onClear={() => onChange('')}
      />
    </Field>
  )
}

function DialogActions({ busy, submitLabel, onCancel }: { busy: boolean; submitLabel: string; onCancel(): void }) {
  return (
    <div className="tmux-dialog-actions">
      <Button type="button" variant="ghost" size="sm" onClick={onCancel} disabled={busy}>
        Cancel
      </Button>
      <Button type="submit" variant="primary" size="sm" disabled={busy}>
        {busy ? 'Working…' : submitLabel}
      </Button>
    </div>
  )
}

export interface NewSessionValues {
  name: string
  cwd: string
  command: string
  window_name: string
}

export function NewSessionDialog({
  recentDirectories,
  onSubmit,
  onClose,
}: {
  recentDirectories: readonly string[]
  onSubmit(values: NewSessionValues): Promise<void>
  onClose(): void
}) {
  const ids = useId()
  const [values, setValues] = useState<NewSessionValues>({ name: '', cwd: '', command: '', window_name: '' })
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const nameError = validateName(values.name)
  const windowError = validateName(values.window_name)
  const cwdError = validateDirectory(values.cwd)
  const commandError = validateCommand(values.command)
  const invalid = Boolean(nameError || windowError || cwdError || commandError)
  const set = (key: keyof NewSessionValues) => (next: string) => setValues((current) => ({ ...current, [key]: next }))

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (invalid || busy) return
    setBusy(true)
    setFailure(null)
    try {
      await onSubmit(values)
      onClose()
    } catch (error) {
      setFailure(errorMessage(error))
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="tmux-dialog">
        <DialogTitle>New session</DialogTitle>
        <DialogDescription>Start a detached tmux session; the console terminal moves to it.</DialogDescription>
        <form className="tmux-dialog-form" onSubmit={(event) => void submit(event)}>
          <Field id={`${ids}-name`} label="Name" hint="Empty lets tmux number it." error={nameError}>
            <Input
              id={`${ids}-name`}
              value={values.name}
              onChange={set('name')}
              placeholder="api"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={nameError ? true : undefined}
            />
          </Field>
          <DirectoryField
            id={`${ids}-cwd`}
            value={values.cwd}
            onChange={set('cwd')}
            recent={recentDirectories}
            error={cwdError}
          />
          <Field
            id={`${ids}-command`}
            label="Command"
            hint="Runs instead of your shell, e.g. claude. The window closes when it exits."
            error={commandError}
          >
            <Input
              id={`${ids}-command`}
              className="tmux-mono"
              value={values.command}
              onChange={set('command')}
              placeholder="Your default shell"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={commandError ? true : undefined}
            />
          </Field>
          <Field id={`${ids}-window`} label="First window name" error={windowError}>
            <Input
              id={`${ids}-window`}
              value={values.window_name}
              onChange={set('window_name')}
              placeholder="Named after its command"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={windowError ? true : undefined}
            />
          </Field>
          {failure ? (
            <p className={uiClasses.fieldError} role="alert">
              {failure}
            </p>
          ) : null}
          <DialogActions busy={busy} submitLabel="Create session" onCancel={onClose} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

export interface NewWindowValues {
  name: string
  cwd: string
  command: string
  select: boolean
}

export function NewWindowDialog({
  sessionName,
  recentDirectories,
  onSubmit,
  onClose,
}: {
  sessionName: string
  recentDirectories: readonly string[]
  onSubmit(values: NewWindowValues): Promise<void>
  onClose(): void
}) {
  const ids = useId()
  const [values, setValues] = useState<NewWindowValues>({ name: '', cwd: '', command: '', select: true })
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const nameError = validateName(values.name)
  const cwdError = validateDirectory(values.cwd)
  const commandError = validateCommand(values.command)
  const invalid = Boolean(nameError || cwdError || commandError)
  const set = (key: 'name' | 'cwd' | 'command') => (next: string) =>
    setValues((current) => ({ ...current, [key]: next }))

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (invalid || busy) return
    setBusy(true)
    setFailure(null)
    try {
      await onSubmit(values)
      onClose()
    } catch (error) {
      setFailure(errorMessage(error))
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="tmux-dialog">
        <DialogTitle>New window</DialogTitle>
        <DialogDescription>
          A new window in <span className="tmux-mono">{sessionName}</span>.
        </DialogDescription>
        <form className="tmux-dialog-form" onSubmit={(event) => void submit(event)}>
          <Field id={`${ids}-name`} label="Name" hint="Empty names it after its command." error={nameError}>
            <Input
              id={`${ids}-name`}
              value={values.name}
              onChange={set('name')}
              placeholder="logs"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={nameError ? true : undefined}
            />
          </Field>
          <DirectoryField
            id={`${ids}-cwd`}
            value={values.cwd}
            onChange={set('cwd')}
            recent={recentDirectories}
            error={cwdError}
          />
          <Field
            id={`${ids}-command`}
            label="Command"
            hint="Runs instead of your shell, e.g. iii compose --up."
            error={commandError}
          >
            <Input
              id={`${ids}-command`}
              className="tmux-mono"
              value={values.command}
              onChange={set('command')}
              placeholder="Your default shell"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={commandError ? true : undefined}
            />
          </Field>
          <Checkbox
            label="Switch to it — every terminal on this session follows"
            checked={values.select}
            onChange={(event) => setValues((current) => ({ ...current, select: event.currentTarget.checked }))}
          />
          {failure ? (
            <p className={uiClasses.fieldError} role="alert">
              {failure}
            </p>
          ) : null}
          <DialogActions busy={busy} submitLabel="Create window" onCancel={onClose} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function RenameDialog({
  title,
  current,
  onSubmit,
  onClose,
}: {
  title: string
  current: string
  onSubmit(name: string): Promise<void>
  onClose(): void
}) {
  const id = useId()
  const [name, setName] = useState(current)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const nameError = name.trim() === '' ? 'Enter a name.' : validateName(name)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (nameError || busy) return
    if (name.trim() === current) {
      onClose()
      return
    }
    setBusy(true)
    setFailure(null)
    try {
      await onSubmit(name.trim())
      onClose()
    } catch (error) {
      setFailure(errorMessage(error))
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="tmux-dialog">
        <DialogTitle>{title}</DialogTitle>
        <form className="tmux-dialog-form" onSubmit={(event) => void submit(event)}>
          <Field id={id} label="Name" error={name.trim() === '' ? null : nameError}>
            <Input
              id={id}
              value={name}
              onChange={setName}
              autoComplete="off"
              spellCheck={false}
              autoFocus
              aria-invalid={nameError && name.trim() !== '' ? true : undefined}
            />
          </Field>
          {failure ? (
            <p className={uiClasses.fieldError} role="alert">
              {failure}
            </p>
          ) : null}
          <DialogActions busy={busy} submitLabel="Rename" onCancel={onClose} />
        </form>
      </DialogContent>
    </Dialog>
  )
}
