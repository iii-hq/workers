import { Settings } from 'lucide-react'
import { useEffect, useState } from 'react'
import { ProviderSettingsDialog } from '@/components/providers/ProviderSettingsDialog'
import {
  type ActiveProvider,
  isLocalProvider,
  PROVIDER_DISPLAY,
} from '@/components/providers/provider-registry'
import { StatusBadge } from '@/components/providers/StatusBadge'
import { useAuthStatus, useProviderConfig } from '@/hooks/use-providers'
import { cn } from '@/lib/utils'

interface ProviderRowProps {
  id: ActiveProvider
  envVar: string
  /**
   * Position in the active providers list (0-based). Displayed as a
   * 1-based shortcut number prefix on the row so the keyboard binding
   * (press N to open provider N) is self-documenting — no hidden
   * footnote needed.
   */
  index?: number
}

/**
 * One row per active provider. The entire row is the affordance: a single
 * `<button>` opens `ProviderSettingsDialog`, which owns every mutation
 * (set / clear key, set / clear runtime overrides, remove credential).
 *
 * The gear icon at the trailing edge is decorative and reinforces the
 * "click to configure" semantic; making the whole row clickable resolved
 * the discoverability issue where a 14px ghost gear was the only entry
 * point operators could reach.
 *
 * For local providers (lmstudio, llamacpp) the env-var column is replaced
 * with `[local]` — the env var is meaningless for providers that don't
 * require a key, and the full env var name is still visible inside the
 * dialog for operators who need it.
 *
 * The `savedFlash` cue is preserved: when the dialog reports a successful
 * mutation we briefly tint the row so the change is noticeable even with
 * the dialog still mounted.
 */
export function ProviderRow({ id, envVar, index }: ProviderRowProps) {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [savedFlash, setSavedFlash] = useState(false)
  const statusQuery = useAuthStatus(id)
  const configQuery = useProviderConfig(id)
  const local = isLocalProvider(id)
  const displayName = PROVIDER_DISPLAY[id] ?? id
  // Row-level signal that runtime overrides (api url / max tokens) are
  // active for this provider. Without this, operators with a custom base
  // URL set weeks ago had no surface indicator that requests were routing
  // somewhere non-default — they had to open the dialog AND expand
  // advanced to discover it. The dialog already uses the same query;
  // React Query dedupes by key so this adds no extra fetches.
  const hasOverride = !!(
    configQuery.data?.default_api_url || configQuery.data?.default_max_tokens
  )

  useEffect(() => {
    if (!savedFlash) return
    const t = setTimeout(() => setSavedFlash(false), 650)
    return () => clearTimeout(t)
  }, [savedFlash])

  // H7: number-key shortcuts on the Configuration page dispatch a window
  // event with the target provider id. Each row listens for its own id.
  // Lightweight cross-component wiring without lifting dialog state.
  useEffect(() => {
    function onOpenRequest(event: Event) {
      const detail = (event as CustomEvent<string>).detail
      if (detail === id) setDialogOpen(true)
    }
    window.addEventListener('iii:open-provider', onOpenRequest)
    return () => window.removeEventListener('iii:open-provider', onOpenRequest)
  }, [id])

  return (
    <div
      className={cn(
        'border-b border-rule last:border-b-0',
        savedFlash && 'trace-flash',
      )}
    >
      <button
        type="button"
        data-provider-row
        onClick={() => setDialogOpen(true)}
        aria-label={`configure ${id}${index != null ? ` (shortcut ${index + 1})` : ''}`}
        className="group w-full text-left flex items-center gap-4 py-3 -mx-3 px-3 hover:bg-rule/40 transition-colors focus:outline-none focus-visible:bg-rule/40"
      >
        <span className="font-mono text-[13px] text-ink w-36 shrink-0 truncate flex items-center gap-2">
          {index != null ? (
            <kbd
              className="font-mono text-[10px] text-ink bg-panel border border-rule px-1.5 leading-none py-0.5 shrink-0 rounded-sm shadow-[0_1px_0_0_rgb(0_0_0_/_0.04)]"
              title={`press ${index + 1} to open`}
            >
              {index + 1}
            </kbd>
          ) : null}
          {displayName}
        </span>
        {local && !statusQuery.data?.configured ? (
          // Local-and-unconfigured: show `[local]` to telegraph "this
          // provider doesn't need a key here." Once a key is set (via
          // env or stored), the env-var column reverts to showing the
          // actual var name — at that point the status badge ([env] or
          // [stored]) carries the "configured" signal, and the env var
          // name is the meaningful identifier in this column.
          <span
            className="font-mono text-[11px] text-ink-ghost w-44 shrink-0"
            title="local server; no api key needed (base url is configurable in advanced)"
          >
            [local]
          </span>
        ) : (
          <span
            className="font-mono text-[11px] text-ink-faint w-44 shrink-0 truncate"
            title={envVar}
          >
            {envVar}
          </span>
        )}
        <span className="flex-1 min-w-0 flex items-center gap-2">
          <StatusBadge
            status={statusQuery.data}
            isLoading={statusQuery.isLoading}
            isError={statusQuery.isError}
            envVar={envVar}
          />
          {hasOverride ? (
            // Single-word badge matching `[stored]` / `[env]` / `[local]`
            // disclosure shape. Axes + values both live in the title
            // tooltip; the visible badge is purely a signal that an
            // override exists. Dialog is the place to inspect details
            // — same as how `[stored]` doesn't render the masked key
            // value inline.
            <span
              className="font-mono text-[11px] text-ink shrink-0"
              title={
                [
                  configQuery.data?.default_api_url
                    ? `endpoint = ${configQuery.data.default_api_url}`
                    : null,
                  configQuery.data?.default_max_tokens != null
                    ? `max tokens = ${configQuery.data.default_max_tokens}`
                    : null,
                ]
                  .filter(Boolean)
                  .join(' · ') || 'runtime overrides active'
              }
            >
              [overridden]
            </span>
          ) : null}
        </span>
        <Settings
          size={14}
          aria-hidden
          className="shrink-0 text-ink-ghost group-hover:text-ink-faint transition-colors"
        />
      </button>

      <ProviderSettingsDialog
        provider={id}
        envVar={envVar}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onMutated={() => setSavedFlash(true)}
      />
    </div>
  )
}
