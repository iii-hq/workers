import {
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react'
import { Button } from '@/components/shadcn/button'
import { Checkbox } from '@/components/shadcn/checkbox'
import { Dialog } from '@/components/shadcn/dialog'
import { Input } from '@/components/shadcn/input'
import {
  FormDialogContent,
  FormDialogDescription,
  FormDialogTitle,
} from '@/components/ui/FormDialog'
import {
  addAccount,
  type ClaudeAccount,
  parseAccountsError,
  removeAccount,
  updateAccount,
} from '@/lib/backend/claude-accounts'
import {
  ACCOUNT_COLORS,
  ADD_EXPLAIN,
  ADD_TITLE,
  COLOR_LABEL,
  CONTINUE_TO_SIGN_IN,
  colorName,
  colorTitle,
  DELETE_FOLDER,
  MAX_NAME,
  NAME_LABEL,
  NAME_PLACEHOLDER,
  nameProblem,
  nextColor,
  RENAME_BUTTON,
  RENAME_TITLE,
  removeButton,
  removeDescription,
  removeTitle,
} from '@/lib/claude-accounts-words'
import { cn } from '@/lib/utils'
import { AccountDot } from './AccountCard'

/** The five colours as swatches, the chosen one ringed (ColorChoice). */
function ColorChoice({
  value,
  onChange,
  labelledBy,
}: {
  value: string
  onChange: (token: string) => void
  labelledBy: string
}) {
  return (
    <div
      role="radiogroup"
      aria-labelledby={labelledBy}
      className="flex items-center gap-2"
    >
      {ACCOUNT_COLORS.map((token) => {
        const selected = token === value
        return (
          // biome-ignore lint/a11y/useSemanticElements: a swatch row, as the project dialog's
          <button
            key={token}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={colorName(token)}
            data-account-swatch={token}
            onClick={() => onChange(token)}
            className={cn(
              'flex size-8 items-center justify-center rounded-full focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:size-11',
              selected ? 'bg-surface-selected' : 'hover:bg-surface-hover',
            )}
          >
            <AccountDot color={token} className="size-4" />
          </button>
        )
      })}
    </div>
  )
}

function Shell({
  open,
  onOpenChange,
  title,
  description,
  children,
  initialFocus,
  name,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  children: ReactNode
  initialFocus?: RefObject<HTMLInputElement | null>
  name: string
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <FormDialogContent
        data-account-dialog={name}
        className="w-[min(28rem,calc(100vw-2rem))] [--dialog-max-width:28rem]"
        initialFocus={initialFocus}
      >
        <div className="flex flex-col gap-4 p-5">
          <div className="flex flex-col gap-1 pr-8">
            <FormDialogTitle>{title}</FormDialogTitle>
            {description ? (
              <FormDialogDescription>{description}</FormDialogDescription>
            ) : null}
          </div>
          {children}
        </div>
      </FormDialogContent>
    </Dialog>
  )
}

/**
 * "Add a Claude Code account": the name and the colour, then the sign-in
 * (`onAdded` opens it). The worker's id is the name's slug.
 */
export function AddAccountDialog({
  open,
  onOpenChange,
  accounts,
  onAdded,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  accounts: readonly ClaudeAccount[]
  onAdded: (account: ClaudeAccount) => void
}) {
  const [name, setName] = useState('')
  const [color, setColor] = useState<string>('blue')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const ref = useRef<HTMLInputElement>(null)
  const ids = useId()
  // biome-ignore lint/correctness/useExhaustiveDependencies: seeded once per open
  useEffect(() => {
    if (!open) return
    setName('')
    setColor(nextColor(accounts))
    setError(null)
    setBusy(false)
  }, [open])
  const problem = nameProblem(name, accounts)
  const submit = async () => {
    const trimmed = name.trim()
    if (busy || !trimmed || problem) return
    setBusy(true)
    setError(null)
    try {
      const account = await addAccount({ name: trimmed, color })
      onOpenChange(false)
      onAdded(account)
    } catch (err) {
      setError(parseAccountsError(err).message)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Shell
      open={open}
      onOpenChange={onOpenChange}
      title={ADD_TITLE}
      initialFocus={ref}
      name="add"
    >
      <form
        noValidate
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
      >
        <div className="flex flex-col gap-1.5">
          <label
            htmlFor={`${ids}-name`}
            className="text-body font-medium text-ink"
          >
            {NAME_LABEL}
          </label>
          <Input
            ref={ref}
            id={`${ids}-name`}
            name="account-name"
            value={name}
            maxLength={MAX_NAME + 8}
            autoComplete="off"
            spellCheck={false}
            placeholder={NAME_PLACEHOLDER}
            aria-invalid={problem ? true : undefined}
            onChange={(e) => setName(e.currentTarget.value)}
          />
          {problem ? (
            <p role="alert" className="text-xs text-alert">
              {problem}
            </p>
          ) : null}
        </div>
        <div className="flex flex-col gap-2">
          <span id={`${ids}-color`} className="text-body font-medium text-ink">
            {COLOR_LABEL}
          </span>
          <ColorChoice
            value={color}
            onChange={setColor}
            labelledBy={`${ids}-color`}
          />
        </div>
        <p className="text-xs text-ink-faint">{ADD_EXPLAIN}</p>
        {error ? (
          <p role="alert" className="text-sm text-alert">
            {error}
          </p>
        ) : null}
        <div className="flex items-center justify-end gap-2">
          <Button
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            disabled={busy || !name.trim() || Boolean(problem)}
          >
            {CONTINUE_TO_SIGN_IN}
          </Button>
        </div>
      </form>
    </Shell>
  )
}

/** "Rename account". */
export function RenameAccountDialog({
  account,
  accounts,
  onClose,
}: {
  account: ClaudeAccount | null
  accounts: readonly ClaudeAccount[]
  onClose: () => void
}) {
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const ref = useRef<HTMLInputElement>(null)
  const ids = useId()
  useEffect(() => {
    if (!account) return
    setName(account.name)
    setError(null)
    setBusy(false)
  }, [account])
  if (!account) return null
  const problem = nameProblem(name, accounts, account.id)
  const submit = async () => {
    const trimmed = name.trim()
    if (busy || !trimmed || problem) return
    if (trimmed === account.name) {
      onClose()
      return
    }
    setBusy(true)
    try {
      await updateAccount({ account: account.id, name: trimmed })
      onClose()
    } catch (err) {
      setError(parseAccountsError(err).message)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Shell
      open
      onOpenChange={(open) => !open && onClose()}
      title={RENAME_TITLE}
      initialFocus={ref}
      name="rename"
    >
      <form
        noValidate
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
      >
        <div className="flex flex-col gap-1.5">
          <label
            htmlFor={`${ids}-name`}
            className="text-body font-medium text-ink"
          >
            {NAME_LABEL}
          </label>
          <Input
            ref={ref}
            id={`${ids}-name`}
            name="account-name"
            value={name}
            maxLength={MAX_NAME + 8}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={problem || error ? true : undefined}
            onChange={(e) => setName(e.currentTarget.value)}
          />
          {problem || error ? (
            <p role="alert" className="text-xs text-alert">
              {problem ?? error}
            </p>
          ) : null}
        </div>
        <div className="flex items-center justify-end gap-2">
          <Button type="button" variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            disabled={busy || !name.trim() || Boolean(problem)}
          >
            {RENAME_BUTTON}
          </Button>
        </div>
      </form>
    </Shell>
  )
}

/** "Color of Personal": a click on a swatch saves it. */
export function AccountColorDialog({
  account,
  onClose,
}: {
  account: ClaudeAccount | null
  onClose: () => void
}) {
  const [error, setError] = useState<string | null>(null)
  const ids = useId()
  useEffect(() => {
    if (account) setError(null)
  }, [account])
  if (!account) return null
  return (
    <Shell
      open
      onOpenChange={(open) => !open && onClose()}
      title={colorTitle(account.name)}
      name="color"
    >
      <span id={`${ids}-color`} className="sr-only">
        {COLOR_LABEL}
      </span>
      <ColorChoice
        value={account.color}
        labelledBy={`${ids}-color`}
        onChange={(token) => {
          if (token === account.color) {
            onClose()
            return
          }
          updateAccount({ account: account.id, color: token })
            .then(onClose)
            .catch((err: unknown) => setError(parseAccountsError(err).message))
        }}
      />
      {error ? (
        <p role="alert" className="text-sm text-alert">
          {error}
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
      </div>
    </Shell>
  )
}

/** "Remove Personal?", with "Also delete its folder". */
export function RemoveAccountDialog({
  account,
  onClose,
}: {
  account: ClaudeAccount | null
  onClose: () => void
}) {
  const [deleteFiles, setDeleteFiles] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const ids = useId()
  useEffect(() => {
    if (!account) return
    setDeleteFiles(false)
    setError(null)
    setBusy(false)
  }, [account])
  if (!account) return null
  return (
    <Shell
      open
      onOpenChange={(open) => !open && !busy && onClose()}
      title={removeTitle(account.name)}
      description={removeDescription(deleteFiles)}
      name="remove"
    >
      <div className="flex items-start gap-2.5 text-sm text-ink">
        <Checkbox
          id={`${ids}-delete`}
          checked={deleteFiles}
          onCheckedChange={(checked) => setDeleteFiles(checked === true)}
        />
        <label htmlFor={`${ids}-delete`} className="flex min-w-0 flex-col">
          <span>{DELETE_FOLDER}</span>
          {account.dirDisplay ? (
            <span className="truncate font-mono text-xs text-ink-faint">
              {account.dirDisplay}
            </span>
          ) : null}
        </label>
      </div>
      {error ? (
        <p role="alert" className="text-sm text-alert">
          {error}
        </p>
      ) : null}
      <div className="flex items-center justify-end gap-2">
        <Button variant="secondary" disabled={busy} onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="destructive"
          disabled={busy}
          onClick={() => {
            setBusy(true)
            setError(null)
            removeAccount(account.id, deleteFiles)
              .then(onClose)
              .catch((err: unknown) =>
                setError(parseAccountsError(err).message),
              )
              .finally(() => setBusy(false))
          }}
        >
          {removeButton(deleteFiles)}
        </Button>
      </div>
    </Shell>
  )
}
