/**
 * Session chip: the chat header's read-out of which system prompt this
 * session runs under, and a dialog showing the prompt itself.
 *
 * Read-only by design. A session's system prompt is chosen once, on the
 * new-session screen, before the first message — this surface reports what
 * that choice resolved to, it never changes it.
 *
 * It lives here rather than in the console because this worker owns system
 * prompts — the console has no business knowing what a `system-prompts/` file
 * is. The chip is injected into the header's `chat` slot, the same slot the
 * harness uses for its context meter.
 *
 * A chip receives only `{ sessionId, modelId, contextWindow }`, so the active
 * choice is read from the session's own metadata (`system_prompt`, written by
 * the console's new-session picker) and kept current by two events —
 * `session::meta-updated` and `session::created`, see the subscribe effect for
 * why both are needed.
 *
 * The dialog asks the harness to preview the same deterministic construction
 * used by a turn, split into built-in, selected, and runtime-injected layers.
 * It does not need a model request to exist first.
 */

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  type SessionChipProps,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'

/** Namespaced so the per-session function ids can't collide with another
 *  worker's chip subscriptions. */
const META_FN = 'iii::directory-ui::sysprompt-meta'
const CREATED_FN = 'iii::directory-ui::sysprompt-created'

type Strategy = 'enrich' | 'replace'

interface ActivePrompt {
  name: string
  strategy: Strategy
  /** Body captured when the session picked it — this is what the console
   * sends, even if the source file changes later. */
  snapshotBody: string
}

interface SessionPromptContext {
  active: ActivePrompt | null
  mode?: 'ask' | 'agent'
  filesystemRoot?: string
}

type PromptPartKind =
  | 'built_in'
  | 'selected'
  | 'runtime'
  | 'registry_notice'
  | 'injected'

interface SystemPromptPreview {
  parts: Array<{
    kind: PromptPartKind
    name?: string
    body: string
  }>
}

/** Session metadata is untrusted wire JSON; anything unreadable means "no
 *  system prompt", never a half-built chip. */
function readPromptContext(metadata: unknown): SessionPromptContext {
  if (typeof metadata !== 'object' || metadata === null) return { active: null }
  const root = metadata as Record<string, unknown>
  const fsScope = root.fs_scope
  const filesystemRoot =
    typeof fsScope === 'object' &&
    fsScope !== null &&
    typeof (fsScope as Record<string, unknown>).root === 'string'
      ? ((fsScope as Record<string, unknown>).root as string)
      : undefined
  const mode =
    root.mode === 'ask' || root.mode === 'agent' ? root.mode : undefined
  const sp = root.system_prompt
  if (typeof sp !== 'object' || sp === null)
    return { active: null, mode, filesystemRoot }
  const md = sp as Record<string, unknown>
  const choice = md.choice
  const name =
    typeof choice === 'object' &&
    choice !== null &&
    typeof (choice as Record<string, unknown>).named === 'string'
      ? ((choice as Record<string, unknown>).named as string)
      : null
  if (!name) return { active: null, mode, filesystemRoot }
  return {
    active: {
      name,
      strategy: md.strategy === 'override' ? 'replace' : 'enrich',
      snapshotBody: typeof md.named_body === 'string' ? md.named_body : '',
    },
    mode,
    filesystemRoot,
  }
}

/** Catalog model keys are `<provider>::<model>`; the router keys its identity
 * prompt by that provider. */
function providerOf(modelId: string | undefined): string | undefined {
  if (!modelId) return undefined
  const separator = modelId.indexOf('::')
  return separator > 0 ? modelId.slice(0, separator) : undefined
}

const FETCH_FAILED =
  '_Could not build the system prompt preview — close and reopen to retry._'

function partLabel(
  kind: PromptPartKind,
  strategy: Strategy | undefined,
): string {
  if (kind === 'built_in') return 'built-in'
  if (kind === 'selected')
    return strategy === 'replace' ? 'replaces built-in' : 'appended'
  if (kind === 'runtime') return 'session context'
  if (kind === 'registry_notice') return 'registry update'
  return 'injected'
}

/** Clipboard write that survives http://<LAN-IP> (insecure context, where
 *  navigator.clipboard is undefined) — the console's lib/clipboard strategy,
 *  inlined because injected bundles only get components from
 *  @iii-dev/console-ui, not its libs. */
async function copyText(text: string): Promise<boolean> {
  if (typeof navigator !== 'undefined' && navigator.clipboard) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // Permissions can reject even on secure origins — try the fallback.
    }
  }
  const textarea = document.createElement('textarea')
  textarea.value = text
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.left = '-9999px'
  document.body.appendChild(textarea)
  textarea.select()
  let ok = false
  try {
    ok = document.execCommand('copy')
  } catch {
    ok = false
  }
  textarea.remove()
  return ok
}

/** chars/4 — the usual rough-tokens rule of thumb, labeled approximate. The
 *  prompt is the largest fixed spend in every turn's window; the surface
 *  that displays it should put a number on it. */
function sizeLabel(body: string): string {
  const tokens = Math.max(1, Math.round(body.length / 4))
  return tokens >= 1000
    ? `~${(tokens / 1000).toFixed(1)}k tokens`
    : `~${tokens} tokens`
}

/**
 * A pinned-feeling header (role · name · size, plus a copy affordance) over
 * the rendered document. `synthetic`
 * marks bodies that are fallback MESSAGES rather than prompt text — those
 * hide copy and size, which would be nonsense about the message itself.
 */
function PromptPart({
  label,
  name,
  body,
  synthetic,
}: {
  /** Role word, uppercased by CSS. */
  label: string
  /** Data, never uppercased: a prompt filename or a provider id. */
  name?: string
  body: string | null
  synthetic?: boolean
}) {
  const [copied, setCopied] = useState(false)
  const handleCopy = useCallback(() => {
    if (body === null) return
    void copyText(body).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }, [body])

  const real = body !== null && !synthetic
  return (
    <section className="dir-ui-sysprompt-partwrap">
      <div className="dir-ui-sysprompt-parthead">
        <h3 className="dir-ui-sysprompt-part">
          <span className="dir-ui-sysprompt-partrole">{label}</span>
          {name ? (
            <span className="dir-ui-sysprompt-partname"> · {name}</span>
          ) : null}
          {real ? (
            <span className="dir-ui-sysprompt-partmeta">
              {' '}
              · {sizeLabel(body)}
            </span>
          ) : null}
        </h3>
        {real ? (
          <button
            type="button"
            className="dir-ui-sysprompt-copy"
            onClick={handleCopy}
            data-copied={copied || undefined}
          >
            {copied ? 'copied' : 'copy'}
          </button>
        ) : null}
      </div>
      {body === null ? (
        // role="status" announces the wait; the reserved height (CSS) keeps
        // the centered dialog from teleporting when content lands.
        <p className="dir-ui-sysprompt-loading" role="status">
          loading…
        </p>
      ) : body.trim() === '' ? (
        <p className="dir-ui-sysprompt-empty">this prompt is empty.</p>
      ) : (
        <pre className="dir-ui-sysprompt-body">{body}</pre>
      )}
    </section>
  )
}

interface SessionMetaEvent {
  session_id?: string
  metadata?: Record<string, unknown>
}

export function createSystemPromptChip(host: Host) {
  return function SystemPromptChip({ sessionId, modelId }: SessionChipProps) {
    const [context, setContext] = useState<SessionPromptContext>({
      active: null,
    })
    const [open, setOpen] = useState(false)
    const [preview, setPreview] = useState<SystemPromptPreview | null>(null)
    const [notice, setNotice] = useState<string | null>(null)
    const active = context.active

    const readMeta = useCallback(
      () =>
        host.iii.trigger<{
          meta?: { metadata?: Record<string, unknown> }
        } | null>('session::get', {
          session_id: sessionId,
        }),
      [sessionId],
    )

    // Hydrate from the session's stored metadata.
    useEffect(() => {
      let cancelled = false
      setContext({ active: null })
      setOpen(false)
      readMeta()
        // A draft session does not exist server-side yet; nothing to show
        // until the first send materialises it (see the `created` binding).
        .then((res) => {
          if (!cancelled) setContext(readPromptContext(res?.meta?.metadata))
        })
        .catch(() => {})
      return () => {
        cancelled = true
      }
    }, [readMeta])

    /*
     * Two events, because the choice can be made BEFORE the session exists.
     *
     *   meta-updated — carries the new metadata, so read it straight off.
     *   created      — carries none (title/status/timestamps only), and this
     *                  is the one that fires on the first send, when a draft's
     *                  pending choice is finally written. Re-read on it, or
     *                  the chip stays blank for the whole session.
     *
     * The function ids carry the session: two chips can be mounted at once,
     * and either teardown would otherwise cancel the other's subscription.
     */
    useEffect(() => {
      const metaFn = `${META_FN}::${sessionId}`
      const createdFn = `${CREATED_FN}::${sessionId}`

      const offMeta = host.iii.on<SessionMetaEvent>(metaFn, (event) => {
        if (!event || event.session_id !== sessionId) return
        setContext(readPromptContext(event.metadata))
      })
      const offCreated = host.iii.on<SessionMetaEvent>(createdFn, (event) => {
        if (!event || event.session_id !== sessionId) return
        readMeta()
          .then((res) => setContext(readPromptContext(res?.meta?.metadata)))
          .catch(() => {})
      })
      const offMetaTrigger = host.iii.registerTrigger({
        type: 'session::meta-updated',
        function_id: `${metaFn}::${host.iii.browserId}`,
        config: {},
      })
      const offCreatedTrigger = host.iii.registerTrigger({
        type: 'session::created',
        function_id: `${createdFn}::${host.iii.browserId}`,
        config: {},
      })
      return () => {
        offMetaTrigger()
        offCreatedTrigger()
        offMeta()
        offCreated()
      }
    }, [sessionId, readMeta])

    const openDialog = useCallback(() => {
      setOpen(true)
      setPreview(null)
      setNotice(null)
      const provider = providerOf(modelId)
      const selectedPrompt = active?.snapshotBody.trim()
        ? {
            name: active.name,
            body: active.snapshotBody,
            strategy:
              active.strategy === 'replace'
                ? ('override' as const)
                : ('enrich' as const),
          }
        : undefined
      host.iii
        .trigger<SystemPromptPreview>('harness::system-prompt::get', {
          session_id: sessionId,
          ...(provider ? { provider } : {}),
          ...(context.mode ? { mode: context.mode } : {}),
          ...(context.filesystemRoot
            ? { filesystem_root: context.filesystemRoot }
            : {}),
          ...(selectedPrompt ? { selected_prompt: selectedPrompt } : {}),
        })
        .then(setPreview)
        .catch(() => setNotice(FETCH_FAILED))
    }, [active, context.filesystemRoot, context.mode, modelId, sessionId])

    const strategy = active?.strategy

    return (
      <>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              className="dir-ui-sysprompt-chip"
              onClick={openDialog}
              aria-haspopup="dialog"
            >
              {/* A prompt name is a filename, not a label: the row's uppercase is
              a style rule and would misreport `pt-BR` as `PT-BR`, so print it
              as authored. `default` is not a filename — it is a state word,
              and it keeps the row's casing like CTX / EXPORT / READY. */}
              {active ? (
                <span className="dir-ui-sysprompt-name">{active.name}</span>
              ) : (
                'default'
              )}
              {/* The strategy as a glyph rather than a word. `+` is "the built-in
              AND this one"; the swap arrows are "this one INSTEAD of it" —
              the two silhouettes differ enough to tell apart at a glance,
              which a pair of same-length uppercase words did not. The word
              itself survives in the tooltip and in the dialog's opening
              sentence, so nothing depends on reading the icon cold. */}
              {strategy ? (
                <svg
                  className="dir-ui-sysprompt-strategy"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  {strategy === 'enrich' ? (
                    <>
                      <path d="M5 12h14" />
                      <path d="M12 5v14" />
                    </>
                  ) : (
                    <>
                      <path d="m16 3 4 4-4 4" />
                      <path d="M20 7H4" />
                      <path d="m8 21-4-4 4-4" />
                      <path d="M4 17h16" />
                    </>
                  )}
                </svg>
              ) : null}
              {/* Says "this opens something" — the row is otherwise all
              read-outs. Drawn inline at lucide's `chevron-down` geometry
              (24 viewBox, 2px round stroke): the console's icon set is not
              a dependency of an injected bundle, but its stroke is. */}
              <svg
                className="dir-ui-sysprompt-caret"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <path d="m6 9 6 6 6-6" />
              </svg>
            </button>
          </TooltipTrigger>
          <TooltipContent>
            {active
              ? `system prompt: ${active.name} (${active.strategy}) — click to read it`
              : 'system prompt: the system default — click to read it'}
          </TooltipContent>
        </Tooltip>

        <Dialog open={open} onOpenChange={setOpen}>
          {/* Re-declare the scope INSIDE the dialog: DialogContent renders
              through a portal to document.body, which lifts it out of the
              `[data-iii-ui]` wrapper the console mounts around injected
              renders — without this, every scoped rule below silently misses
              and the controls render as bare text. */}
          <DialogContent
            data-iii-ui="iii-directory"
            className="dir-ui-sysprompt-dialog"
          >
            <DialogTitle>system prompt</DialogTitle>
            <DialogDescription>
              Preview assembled from this session's settings and declared worker
              injections, in send order. Request-dependent hooks and compaction
              may change the final prompt when a message is sent.
            </DialogDescription>

            {/* ONE labeled scroll region for everything below the description,
                with the title, description and ✕ pinned above it instead of
                scrolling away with the document. */}
            <section
              className="dir-ui-sysprompt-scroll"
              aria-label="system prompt preview"
            >
              {notice ? (
                <PromptPart label="preview" body={notice} synthetic />
              ) : preview ? (
                preview.parts.map((part, index) => (
                  <PromptPart
                    key={`${part.kind}-${index}`}
                    label={partLabel(part.kind, active?.strategy)}
                    name={part.name}
                    body={part.body}
                  />
                ))
              ) : (
                <PromptPart label="building preview" body={null} />
              )}
            </section>
          </DialogContent>
        </Dialog>
      </>
    )
  }
}
