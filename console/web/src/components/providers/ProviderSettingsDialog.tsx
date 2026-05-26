import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, ChevronUp, Eye, EyeOff, X } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  type ActiveProvider,
  isLocalProvider,
  localBaseUrlEnv,
  PROVIDER_DEFAULTS,
  PROVIDER_DISPLAY,
  PROVIDER_DOCS,
  PROVIDER_DOCS_LABEL,
  PROVIDER_KEY_PLACEHOLDER,
} from '@/components/providers/provider-registry'
import { StatusBadge } from '@/components/providers/StatusBadge'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Input } from '@/components/ui/Input'
import {
  useAuthStatus,
  useClearProviderConfig,
  useDeleteToken,
  useProviderConfig,
  useSetProviderConfig,
  useSetToken,
} from '@/hooks/use-providers'
import {
  normalizeErrorMessage,
  validateApiUrl,
  validateMaxTokens,
} from '@/lib/providers'
import { cn } from '@/lib/utils'

interface ProviderSettingsDialogProps {
  provider: ActiveProvider
  envVar: string
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Fires after any successful mutation so the row can flash. */
  onMutated?: () => void
}

/**
 * Errors surfaced inside the dialog always carry a recovery hint. The
 * raw message names what failed; the hint tells the operator what to try
 * next. Different mutations get different hints because the recovery
 * path differs (key format vs URL reachability vs harness liveness).
 */
interface SurfaceError {
  message: string
  hint: string
}

/**
 * Post-destructive-action receipt. Holds the previous state so a `reset
 * overrides` can be undone within the receipt window; `remove key` has
 * no true undo (the credential isn't available client-side) so we only
 * show a receipt — the operator can re-type the key in the field above.
 *
 * `expiresAt` (reset only) drives a visible countdown so operators
 * know how long they have to act. Without it the affordance would be
 * silently ephemeral — operators relying on it would discover its
 * expiry by accident.
 */
type UndoState =
  | {
      kind: 'reset'
      prev: { default_api_url?: string; default_max_tokens?: number }
      expiresAt: number
    }
  | { kind: 'remove' }
  | null

const UNDO_WINDOW_MS = 8000

/**
 * Single dialog with a single primary `save`. Reads the current credential
 * status and stored overrides, lets the user touch any of them in a flat
 * layout, then commits everything that's actually dirty in parallel on
 * save. Advanced fields (base url + max tokens) live behind a "show
 * advanced" disclosure but auto-expand whenever a non-default override is
 * already stored, so the user always sees what's already in effect.
 *
 * Destructive surfaces (`reset overrides`, `remove key`) use an
 * armed-confirm pattern -- one click reveals a confirmation chip, second
 * click commits. Mirrors the `remove credential` pattern from earlier so
 * nothing in this dialog is a one-tap footgun.
 */
export function ProviderSettingsDialog({
  provider,
  envVar,
  open,
  onOpenChange,
  onMutated,
}: ProviderSettingsDialogProps) {
  const qc = useQueryClient()
  const statusQuery = useAuthStatus(provider)
  const status = statusQuery.data
  const isStored = status?.source === 'stored'
  const isConfigured = status?.configured ?? false
  const defaults = PROVIDER_DEFAULTS[provider]
  const docsUrl = PROVIDER_DOCS[provider]

  const configQuery = useProviderConfig(provider)
  const setTokenMut = useSetToken()
  const setConfigMut = useSetProviderConfig()
  const clearConfigMut = useClearProviderConfig()
  const deleteTokenMut = useDeleteToken()

  // Waits for the post-mutation refetch to settle before resolving so the
  // dialog only closes once the row badge will render the new state. Without
  // this, the dialog closes -> row flashes -> badge revalidation lags by one
  // poll cycle, leaving the success moment desynchronized.
  async function waitForFreshStatus() {
    await Promise.all([
      qc.refetchQueries({ queryKey: ['provider', 'status', provider] }),
      qc.refetchQueries({ queryKey: ['provider', 'config', provider] }),
    ])
  }

  // --- form state ---------------------------------------------------------

  const [key, setKey] = useState('')
  const [revealed, setRevealed] = useState(false)
  const [apiUrl, setApiUrl] = useState('')
  const [maxTokens, setMaxTokens] = useState('')
  const [urlError, setUrlError] = useState<string | null>(null)
  const [maxTokensError, setMaxTokensError] = useState<string | null>(null)

  const storedUrl = configQuery.data?.default_api_url ?? ''
  const storedMaxTokens =
    configQuery.data?.default_max_tokens != null
      ? String(configQuery.data.default_max_tokens)
      : ''
  const hasOverride = !!storedUrl || !!storedMaxTokens

  // Populate from server overrides + reset key field every time the dialog
  // re-opens or the provider changes.
  useEffect(() => {
    if (!open) return
    if (configQuery.isLoading) return
    setKey('')
    setRevealed(false)
    setApiUrl(storedUrl)
    setMaxTokens(storedMaxTokens)
    setUrlError(null)
    setMaxTokensError(null)
    setShowAdvanced(hasOverride)
    setArmed(null)
    setUndo(null)
    setTokenMut.reset()
    setConfigMut.reset()
    clearConfigMut.reset()
    deleteTokenMut.reset()
    // We only re-run on (open, provider, configQuery.data) because the
    // mutation hooks change identity on every render. Including them would
    // wipe form state mid-save.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider, configQuery.data, configQuery.isLoading])

  // --- collapsible advanced + armed confirm states -----------------------

  const [showAdvanced, setShowAdvanced] = useState(hasOverride)
  type Armed = 'reset' | 'remove' | null
  const [armed, setArmed] = useState<Armed>(null)
  const [undo, setUndo] = useState<UndoState>(null)
  const [nowTick, setNowTick] = useState(() => Date.now())
  const dialogBodyRef = useRef<HTMLDivElement>(null)

  // Tick every 500ms while a reset undo is active so the countdown
  // label can re-render. Stops the moment the undo is consumed or
  // expires (so we don't burn CPU on a dialog with no undo state).
  useEffect(() => {
    if (undo?.kind !== 'reset') return
    const id = setInterval(() => setNowTick(Date.now()), 500)
    return () => clearInterval(id)
  }, [undo?.kind])

  // Auto-clear the RESET receipt after the undo window closes — only
  // for resets, because that's the action where undo actually does
  // something (re-saves the snapshotted config). For REMOVE there's no
  // true undo (credential isn't kept client-side), so a countdown
  // would be a false-affordance: the operator watches a timer for an
  // action they can't take. Instead the remove receipt persists until
  // the user closes the dialog, framed as actionable guidance ("type
  // the key above to re-add") rather than a recovery window.
  useEffect(() => {
    if (undo?.kind !== 'reset') return
    const t = setTimeout(() => setUndo(null), UNDO_WINDOW_MS)
    return () => clearTimeout(t)
  }, [undo])

  // Stable input ids for proper <label htmlFor> association in Field.
  const keyInputId = useId()
  const urlInputId = useId()
  const tokensInputId = useId()
  // Stable error ids for aria-describedby on the inputs they describe.
  const keyErrorId = useId()
  // ARIA disclosure pattern: button.aria-controls = region.id, so
  // assistive tech can navigate from the toggle to the content directly.
  const advancedRegionId = useId()

  // --- dirty tracking ----------------------------------------------------

  const keyDirty = key.trim().length > 0
  const urlDirty = apiUrl.trim() !== storedUrl
  const tokensDirty = maxTokens.trim() !== storedMaxTokens
  const isDirty = keyDirty || urlDirty || tokensDirty

  const pending =
    setTokenMut.isPending ||
    setConfigMut.isPending ||
    clearConfigMut.isPending ||
    deleteTokenMut.isPending

  // One error memo per mutation so the hint matches the action that
  // actually failed. The previous "first non-null wins" pattern misled
  // operators when `Promise.all` had two ops and only one failed (the
  // hint always pointed to the first-checked mutation, not the real
  // culprit). Now each error renders proximate to the input that
  // produced it (key error under key field, overrides error inside
  // advanced, destructive errors inside the destructive zone).
  const keyError = useMemo<SurfaceError | null>(() => {
    if (!setTokenMut.error) return null
    return {
      message: normalizeErrorMessage(setTokenMut.error),
      hint: 'check the key format. the harness must be running to store credentials.',
    }
  }, [setTokenMut.error])

  const overridesError = useMemo<SurfaceError | null>(() => {
    if (!setConfigMut.error) return null
    return {
      message: normalizeErrorMessage(setConfigMut.error),
      hint: 'verify the url is reachable and the max tokens value is within range.',
    }
  }, [setConfigMut.error])

  const resetError = useMemo<SurfaceError | null>(() => {
    if (!clearConfigMut.error) return null
    return {
      message: normalizeErrorMessage(clearConfigMut.error),
      hint: 'the harness may be unreachable. retry, or refresh to see current state.',
    }
  }, [clearConfigMut.error])

  const removeError = useMemo<SurfaceError | null>(() => {
    if (!deleteTokenMut.error) return null
    return {
      message: normalizeErrorMessage(deleteTokenMut.error),
      hint: 'the credential may still exist. close the dialog and refresh to verify.',
    }
  }, [deleteTokenMut.error])

  // Partial-success detection: when `save()` ran parallel mutations and
  // one succeeded while the other failed, the dialog stays open with
  // an error visible. Without an explicit success signal, the operator
  // sees the badge update to [stored] AND an error in the same dialog
  // and can't tell what actually happened. The success indicators (✓)
  // render only in this partial state so a fully-successful save (which
  // closes the dialog) doesn't flash them.
  const isPartialSuccess =
    (setTokenMut.isSuccess && !!setConfigMut.error) ||
    (!!setTokenMut.error && setConfigMut.isSuccess)

  // If an error came from the advanced section (overrides / reset),
  // auto-expand advanced so the error is actually visible to be
  // scrolled to. Without this, an operator who saved an invalid URL
  // with advanced collapsed would see scroll-to nothing (the error
  // is rendered inside a hidden disclosure).
  useEffect(() => {
    if (overridesError || resetError) {
      setShowAdvanced(true)
    }
  }, [overridesError, resetError])

  // Scroll the first surface error into view when any appears so the
  // operator never has a failure surface below the fold. Errors render
  // proximate to the action that produced them (per-mutation), and this
  // hook makes sure the proximate location is also visible.
  useEffect(() => {
    if (!keyError && !overridesError && !resetError && !removeError) return
    // Defer one tick so the auto-expand effect above has rendered the
    // advanced section before we try to scroll into it.
    const t = setTimeout(() => {
      const el = dialogBodyRef.current?.querySelector('[data-dialog-error]')
      if (el) {
        ;(el as HTMLElement).scrollIntoView({
          behavior: 'smooth',
          block: 'nearest',
        })
      }
    }, 0)
    return () => clearTimeout(t)
  }, [keyError, overridesError, resetError, removeError])

  // --- save (combined) ---------------------------------------------------

  async function save() {
    if (!isDirty) return

    // Validate first so we don't fire any mutation if anything is bad.
    let validatedUrl: string | undefined
    let validatedTokens: number | undefined
    if (urlDirty) {
      const trimmed = apiUrl.trim()
      if (trimmed.length > 0) {
        const v = validateApiUrl(trimmed)
        if (!v.ok) {
          setUrlError(v.error)
          return
        }
        validatedUrl = v.value
      }
    }
    if (tokensDirty) {
      const trimmed = maxTokens.trim()
      if (trimmed.length > 0) {
        const v = validateMaxTokens(trimmed)
        if (!v.ok) {
          setMaxTokensError(v.error)
          return
        }
        validatedTokens = v.value
      }
    }

    // Use Promise.allSettled so a single-op failure doesn't suppress
    // the other op's outcome. With Promise.all, if setToken rejects
    // first, setConfig's rejection becomes an unhandled promise even
    // though we want both errors to surface in their per-mutation
    // memos. allSettled lets each mutation hook record its own
    // success/failure state independently.
    const ops: Promise<unknown>[] = []
    if (keyDirty) {
      ops.push(
        setTokenMut.mutateAsync({
          provider,
          credential: { type: 'api_key', key: key.trim() },
        }),
      )
    }
    if (validatedUrl !== undefined || validatedTokens !== undefined) {
      ops.push(
        setConfigMut.mutateAsync({
          provider,
          ...(validatedUrl !== undefined && { default_api_url: validatedUrl }),
          ...(validatedTokens !== undefined && {
            default_max_tokens: validatedTokens,
          }),
        }),
      )
    }

    const results = await Promise.allSettled(ops)
    if (results.some((r) => r.status === 'rejected')) {
      // Errors surface via per-mutation memos (keyError, overridesError),
      // each scrolled into view. Leave the dialog open for correction.
      // The operator sees what landed AND what didn't.
      return
    }
    // All attempted ops succeeded. Clear any stale "key removed" or
    // "overrides reset" receipt — the operator just re-added the key
    // (or saved new overrides), so the receipt's instruction is done.
    // Without this, after `removeKey → re-type → save` the receipt
    // would still read "key removed. type a new key above..." while
    // the badge above shows [stored], leaving the operator with two
    // contradictory signals.
    setUndo(null)
    // Wait for status refetch BEFORE firing the flash: out-of-order
    // makes the row flash tint a stale badge. With this order the
    // badge already shows the new state when the flash starts, and
    // they land together.
    await waitForFreshStatus()
    onMutated?.()
    onOpenChange(false)
  }

  // --- destructive actions (armed-confirm) -------------------------------

  async function resetOverrides() {
    // Snapshot the previous overrides BEFORE clearing so we can offer
    // true undo (re-saving the snapshotted values).
    const prev = {
      default_api_url: configQuery.data?.default_api_url,
      default_max_tokens: configQuery.data?.default_max_tokens,
    }
    try {
      await clearConfigMut.mutateAsync(provider)
      setArmed(null)
      await waitForFreshStatus()
      onMutated?.()
      // Reset the field inputs to defaults so the dialog matches the
      // newly-cleared server state.
      setApiUrl('')
      setMaxTokens('')
      setUndo({
        kind: 'reset',
        prev,
        expiresAt: Date.now() + UNDO_WINDOW_MS,
      })
    } catch {
      // Error surfaces via the form error memo; keep armed for retry.
    }
  }

  async function undoReset() {
    if (undo?.kind !== 'reset') return
    const { prev } = undo
    setUndo(null)
    try {
      await setConfigMut.mutateAsync({
        provider,
        ...(prev.default_api_url !== undefined && {
          default_api_url: prev.default_api_url,
        }),
        ...(prev.default_max_tokens !== undefined && {
          default_max_tokens: prev.default_max_tokens,
        }),
      })
      await waitForFreshStatus()
      onMutated?.()
      // Repopulate field inputs from the restored values.
      setApiUrl(prev.default_api_url ?? '')
      setMaxTokens(
        prev.default_max_tokens != null ? String(prev.default_max_tokens) : '',
      )
    } catch {
      // Error surfaces via the form error memo.
    }
  }

  async function removeKey() {
    try {
      await deleteTokenMut.mutateAsync(provider)
      setArmed(null)
      await waitForFreshStatus()
      onMutated?.()
      // Don't auto-close. Show a receipt instead so the operator has
      // a recovery window — the key field is empty above and they can
      // re-type if the remove was a mistake. True undo isn't possible:
      // we don't keep the credential client-side.
      setUndo({ kind: 'remove' })
      // Scroll the dialog content to the top so the key field (the
      // re-add target the receipt points at) is back in view. Without
      // this, after a remove from an expanded-advanced dialog the
      // receipt appears below the destructive zone with the key field
      // potentially off-screen — operator reads "type a new key above"
      // with nothing above to type into.
      const scrollContainer = dialogBodyRef.current?.closest('[role="dialog"]')
      if (scrollContainer) scrollContainer.scrollTop = 0
    } catch {
      // Error surfaces via the remove error memo; keep armed for retry.
    }
  }

  // --- render ------------------------------------------------------------

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-md"
        onEscapeKeyDown={(e) => {
          // When an armed-confirm is showing, Escape should disarm it
          // rather than close the entire dialog. Operators reflexively
          // press Escape to cancel the confirm step; without this, they'd
          // also lose any unsaved edits to the form fields above.
          if (armed !== null) {
            e.preventDefault()
            setArmed(null)
          }
        }}
      >
        <div className="flex items-baseline justify-between gap-4">
          <DialogTitle className="font-mono text-[16px] text-ink lowercase tracking-[-0.01em]">
            {PROVIDER_DISPLAY[provider] ?? provider}
          </DialogTitle>
          <span
            className="font-mono text-[11px] text-ink-faint truncate"
            title={envVar}
          >
            {envVar}
          </span>
        </div>
        {/* `DialogDescription` maps to `aria-describedby` on the dialog
            root; using the live status badge here meant screen readers
            announced the loading "dot dot dot" as the dialog's accessible
            description. Keep the description stable + static, and render
            the visual badge in a sibling element. */}
        <DialogDescription className="sr-only">
          credential settings for {provider}
        </DialogDescription>
        <div className="mt-1 flex items-center gap-2">
          <StatusBadge
            status={status}
            isLoading={statusQuery.isLoading}
            isError={statusQuery.isError}
            envVar={envVar}
          />
        </div>

        <div ref={dialogBodyRef} className="mt-5 flex flex-col gap-3">
          <Field label="api key" error={null} htmlFor={keyInputId}>
            <div className="flex items-center gap-2">
              <div className="flex-1">
                <Input
                  id={keyInputId}
                  type={revealed ? 'text' : 'password'}
                  value={key}
                  onChange={setKey}
                  placeholder={
                    isConfigured
                      ? 'leave blank to keep current'
                      : PROVIDER_KEY_PLACEHOLDER[provider]
                  }
                  preserveCase
                  aria-invalid={!!keyError}
                  aria-describedby={keyError ? keyErrorId : undefined}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') save()
                  }}
                />
              </div>
              <Button
                variant="ghost"
                size="icon"
                type="button"
                onClick={() => setRevealed((r) => !r)}
                aria-label={revealed ? 'hide key' : 'reveal key'}
              >
                {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
              </Button>
            </div>
            {/* Error rendered FIRST when present — operators correcting
                a failed save shouldn't have to scroll past contextual
                notes (env-precedence, docs link, security footnote) to
                find the recovery message. The success indicator follows
                the same priority. */}
            {keyError ? (
              <div
                id={keyErrorId}
                data-dialog-error
                className="mt-2"
                role="alert"
              >
                <p className="font-mono text-[11px] text-alert">
                  error: {keyError.message}
                </p>
                <p className="font-mono text-[11px] text-ink-faint mt-0.5">
                  {keyError.hint}
                </p>
              </div>
            ) : null}
            {isPartialSuccess && setTokenMut.isSuccess ? (
              <p className="font-mono text-[11px] text-ink-faint mt-2">
                <span className="text-accent mr-1">✓</span> key saved
              </p>
            ) : null}
            {status?.source === 'environment' ? (
              keyDirty ? (
                <p className="mt-1.5 font-mono text-[11px] text-ink-faint">
                  note: {envVar} is set in the environment and takes precedence
                  at runtime. saving stores this key as a fallback — it will
                  take effect once the env var is unset.
                </p>
              ) : (
                <p className="mt-1.5 font-mono text-[11px] text-ink-faint">
                  this provider's key currently comes from {envVar}.
                </p>
              )
            ) : null}
            {/* Local-provider context lives in two places by design:
                (1) the per-provider key placeholder above ("any string
                (local server usually accepts anything)") covers the
                no-key-needed case at the input itself; (2) the
                env-var-precedence note inside the advanced section
                covers the LMSTUDIO_BASE_URL / LLAMACPP_BASE_URL
                override. An extra paragraph here would just rephrase
                what the placeholder already says. */}
            <p className="mt-1.5 font-mono text-[11px] text-ink-faint">
              {/* Local providers link to setup docs (not a credentials
                  dashboard) regardless of configured state — "dashboard:"
                  would falsely imply key management for a local server. */}
              {isLocalProvider(provider)
                ? 'docs: '
                : isConfigured
                  ? 'dashboard: '
                  : 'get a key: '}
              <a
                href={`https://${docsUrl}`}
                target="_blank"
                rel="noreferrer noopener"
                className="text-ink hover:text-accent transition-colors"
              >
                {PROVIDER_DOCS_LABEL[provider]}
              </a>
            </p>
            <p className="mt-1 font-mono text-[10px] text-ink-ghost">
              stored on this machine, never sent to the browser after save.
            </p>
          </Field>

          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="mt-1 flex items-center gap-1 font-mono text-[11px] text-ink-faint hover:text-ink transition-colors w-fit"
            aria-expanded={showAdvanced}
            aria-controls={advancedRegionId}
            disabled={configQuery.isLoading}
            title={
              configQuery.isLoading ? 'loading stored overrides...' : undefined
            }
          >
            {showAdvanced ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
            {configQuery.isLoading
              ? 'loading overrides...'
              : showAdvanced
                ? 'hide advanced'
                : 'show advanced'}
          </button>

          {showAdvanced ? (
            <div id={advancedRegionId} className="flex flex-col gap-3 pt-1">
              <p className="font-mono text-[11px] text-ink-faint -mt-1">
                route requests through a proxy or self-hosted endpoint.
                {hasOverride || isStored
                  ? ' destructive actions live below.'
                  : ''}
              </p>
              <Field label="base url" error={urlError} htmlFor={urlInputId}>
                <Input
                  id={urlInputId}
                  // Use type="text" instead of type="url": the latter
                  // triggers browser-native validation that fires before
                  // our `validateApiUrl` and surfaces an inconsistent
                  // error popover (instead of our copy-controlled
                  // `urlError`). Validation logic is the same; the input
                  // type just controls whether the browser intervenes.
                  type="text"
                  value={apiUrl}
                  onChange={(v) => {
                    setApiUrl(v)
                    if (urlError) setUrlError(null)
                  }}
                  placeholder={defaults.default_api_url}
                  preserveCase
                />
                {/* Always-visible default hint so the format / example
                    is reachable even when the field is filled (and the
                    placeholder is therefore suppressed). */}
                <p className="mt-1 font-mono text-[11px] text-ink-faint break-all">
                  default: {defaults.default_api_url}
                </p>
              </Field>
              <Field
                label="max tokens"
                error={maxTokensError}
                htmlFor={tokensInputId}
              >
                <p className="mb-1 font-mono text-[11px] text-ink-faint">
                  ceiling enforced by the harness on tokens the model may
                  generate per request. if the model's own max is lower, the
                  model wins.
                </p>
                <Input
                  id={tokensInputId}
                  type="number"
                  value={maxTokens}
                  onChange={(v) => {
                    setMaxTokens(v)
                    if (maxTokensError) setMaxTokensError(null)
                  }}
                  placeholder={`${defaults.default_max_tokens} (default)`}
                  min={1}
                  max={1_048_576}
                />
              </Field>
              {isLocalProvider(provider) ? (
                <p className="font-mono text-[11px] text-ink-faint -mt-1">
                  local provider · the {localBaseUrlEnv(provider)} env var, if
                  set, overrides this on startup.
                </p>
              ) : null}
              {overridesError ? (
                <div data-dialog-error role="alert">
                  <p className="font-mono text-[11px] text-alert">
                    error: {overridesError.message}
                  </p>
                  <p className="font-mono text-[11px] text-ink-faint mt-0.5">
                    {overridesError.hint}
                  </p>
                </div>
              ) : null}
              {isPartialSuccess && setConfigMut.isSuccess ? (
                <p className="font-mono text-[11px] text-ink-faint">
                  <span className="text-accent mr-1">✓</span> overrides saved
                </p>
              ) : null}

              {/* Destructive zone: both destructive surfaces live behind the
                  advanced disclosure so the dialog footer stays focused on
                  the primary save / cancel actions. Operators who want to
                  reset overrides or remove a key signal that intent by
                  expanding advanced first. */}
              {hasOverride || isStored ? (
                <div className="mt-2 pt-3 border-t border-rule flex flex-col gap-3">
                  {hasOverride ? (
                    armed === 'reset' ? (
                      <ArmedConfirm
                        label="reset overrides to defaults?"
                        confirmLabel="yes, reset"
                        pending={clearConfigMut.isPending}
                        onCancel={() => setArmed(null)}
                        onConfirm={resetOverrides}
                      />
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setArmed('reset')}
                        // Disable while remove is armed so an operator
                        // can't silently replace the armed state by
                        // clicking the other destructive button. They
                        // must explicitly cancel the active confirm
                        // first — no surprise re-arming.
                        disabled={pending || armed === 'remove'}
                        className="self-start text-ink-faint"
                      >
                        reset overrides
                      </Button>
                    )
                  ) : null}
                  {isStored ? (
                    armed === 'remove' ? (
                      <ArmedConfirm
                        label="remove stored key?"
                        confirmLabel="yes, remove"
                        pending={deleteTokenMut.isPending}
                        onCancel={() => setArmed(null)}
                        onConfirm={removeKey}
                        danger
                      />
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setArmed('remove')}
                        // Disabled while reset is armed (see comment on
                        // the reset button above).
                        disabled={pending || armed === 'reset'}
                        className="self-start text-alert hover:text-alert"
                      >
                        remove key
                      </Button>
                    )
                  ) : null}
                  {isStored ? (
                    <p className="font-mono text-[11px] text-ink-faint -mt-1">
                      env-var fallback (if any) still applies after remove.
                    </p>
                  ) : null}
                  {resetError ? (
                    <div data-dialog-error role="alert">
                      <p className="font-mono text-[11px] text-alert">
                        error: {resetError.message}
                      </p>
                      <p className="font-mono text-[11px] text-ink-faint mt-0.5">
                        {resetError.hint}
                      </p>
                    </div>
                  ) : null}
                  {removeError ? (
                    <div data-dialog-error role="alert">
                      <p className="font-mono text-[11px] text-alert">
                        error: {removeError.message}
                      </p>
                      <p className="font-mono text-[11px] text-ink-faint mt-0.5">
                        {removeError.hint}
                      </p>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}
        </div>

        {/* Pinned region: undo receipts and surface errors live directly
            above the footer so they're always proximate to the actions
            that triggered them. `sticky bottom-0` keeps the entire band
            visible even when the dialog body overflows — operator never
            has to scroll to find Save or to read why a save failed.
            Dark-mode bg must match `DialogContent` or the sticky band
            paints the wrong color over scrolled content. */}
        <div className="sticky bottom-0 bg-bg dark:bg-bg-dark mt-6 pt-4 border-t border-rule">
          {undo ? (
            <div
              role="status"
              aria-live="polite"
              className="mb-3 flex items-start justify-between gap-2 px-2 py-1.5 border border-rule"
            >
              <div className="min-w-0">
                <p className="font-mono text-[11px] text-ink">
                  {/* Use a neutral glyph for destructive-action receipts
                      (· before "key removed" / "overrides reset"). The
                      ✓ glyph stays reserved for affirmative successes
                      (`✓ key saved`) so the two signal vocabularies
                      don't collide. */}
                  <span className="text-ink-faint mr-1">·</span>
                  {undo.kind === 'reset'
                    ? 'overrides reset to defaults'
                    : 'key removed'}
                </p>
                {undo.kind === 'remove' ? (
                  <p className="font-mono text-[11px] text-ink-faint mt-0.5">
                    type a new key above to re-add.
                  </p>
                ) : null}
              </div>
              <div className="flex items-center gap-2 shrink-0">
                {undo.kind === 'reset' ? (
                  <button
                    type="button"
                    onClick={undoReset}
                    className="font-mono text-[11px] text-accent hover:text-ink transition-colors"
                  >
                    undo ·{' '}
                    {Math.max(0, Math.ceil((undo.expiresAt - nowTick) / 1000))}s
                    left
                  </button>
                ) : null}
                <button
                  type="button"
                  onClick={() => setUndo(null)}
                  aria-label="dismiss receipt"
                  title="dismiss"
                  className="text-ink-ghost hover:text-ink transition-colors p-0.5"
                >
                  <X size={14} aria-hidden />
                </button>
              </div>
            </div>
          ) : null}
          {/* Errors render at the action that produced them (per-mutation
              proximity); see field-adjacent SurfaceError blocks above.
              Auto-scroll-into-view handles below-the-fold visibility. */}
          <div className="flex items-center justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={save}
              disabled={!isDirty || pending}
            >
              {pending ? 'saving...' : 'save'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/* ---------------------------------------------------------------------- */
/*  Helpers                                                                */
/* ---------------------------------------------------------------------- */

interface FieldProps {
  label: string
  error: string | null
  children: React.ReactNode
  /**
   * When provided, the label becomes a real `<label htmlFor={...}>`
   * binding the visible text to the input. Without it, click-to-focus
   * fails and the visible label is not the accessible name. Pass the
   * matching `id` to the input element rendered as children.
   */
  htmlFor?: string
}

function Field({ label, error, children, htmlFor }: FieldProps) {
  return (
    <div className={cn('flex items-start gap-3')}>
      <label
        htmlFor={htmlFor}
        className="font-mono text-[12px] text-ink w-24 shrink-0 pt-2"
      >
        {label}
      </label>
      <div className="flex-1 min-w-0">
        {children}
        {error ? (
          <p className="mt-1 font-mono text-[11px] text-alert">{error}</p>
        ) : null}
      </div>
    </div>
  )
}

interface ArmedConfirmProps {
  label: string
  confirmLabel: string
  pending: boolean
  onCancel: () => void
  onConfirm: () => void
  danger?: boolean
}

function ArmedConfirm({
  label,
  confirmLabel,
  pending,
  onCancel,
  onConfirm,
  danger,
}: ArmedConfirmProps) {
  // Both buttons are ghost — the destructive yes is tinted in `text-alert`
  // when `danger`, but neither button reads as the dominant CTA. This
  // resolves the "primary visual weight on the destructive action"
  // mismatch where the safe action (no) was a quieter ghost and the
  // confirm was a loud primary. Now the framing is: "two equally weighted
  // options, one of which is clearly destructive."
  //
  // `role="group"` + `aria-label` groups the two confirm buttons with
  // their question text as an accessible name. (Earlier `role="alertdialog"`
  // claimed modal semantics this widget doesn't actually implement — it
  // doesn't trap focus, it's inline within the dialog body.) The autoFocus
  // on `no` still gives screen reader users the safe default after the
  // group becomes the focused context.
  return (
    <span
      role="group"
      aria-label={label}
      className="flex items-center gap-2 font-mono text-[12px] text-ink"
    >
      <span className={cn(danger && 'text-alert')}>{label}</span>
      <Button
        variant="ghost"
        size="sm"
        autoFocus
        onClick={onCancel}
        disabled={pending}
      >
        no
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={onConfirm}
        disabled={pending}
        className={cn(danger && 'text-alert hover:text-alert')}
      >
        {pending ? 'working...' : confirmLabel}
      </Button>
    </span>
  )
}
