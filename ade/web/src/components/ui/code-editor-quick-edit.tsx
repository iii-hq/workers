/**
 * Quick Edit — the editor's inline "change this with AI" prompt (⌘I on a
 * selection), the way Cursor does it: a card opens above the selection with
 * a one-line instruction box, the caller's model rewrites the code, the edit
 * lands in the buffer (undoable) with the changed lines tinted, and the card
 * turns into Accept / Reject.
 *
 * This file holds the request shape, the card and the pure helpers; the
 * Monaco plumbing (view zone, content widget, decorations) lives in
 * `CodeEditor`.
 */

import { ArrowUp, ChevronDown, LoaderCircle, Square, X } from 'lucide-react'
import * as React from 'react'
import { ModelPicker } from '@/components/chat/ModelPicker'
import { Button } from '@/components/shadcn/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Hint } from '@/components/ui/Hint'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { formatBinding } from '@/lib/keybindings/bindings'
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { cn } from '@/lib/utils'

/** What Quick Edit rewrites: the selection, or the whole file around it. */
export type CodeEditorQuickEditScope = 'selection' | 'file'

export interface CodeEditorQuickEditRequest {
  /** What the person asked for, trimmed. */
  instruction: string
  scope: CodeEditorQuickEditScope
  /** The selection the prompt was opened on, 1-based like the gutter. */
  selection: {
    startLine: number
    startColumn: number
    endLine: number
    endColumn: number
    text: string
  }
  /** The whole buffer as it was when the prompt was sent. */
  value: string
  /** Monaco language id of the buffer. */
  language: string
  /** The model picked in the card (`provider::model`); null = the caller's default. */
  model: string | null
  /** The reasoning effort picked with it; `default` = the model's own. */
  thinkingLevel: string
  /** Aborts when the person stops or closes the prompt; a late answer is dropped either way. */
  signal: AbortSignal
}

/** The caller's side of Quick Edit: the model call and its labels. */
export interface CodeEditorQuickEdit {
  /** Text that replaces the selection (or the whole file, for `file`). Reject to show an error. */
  run(request: CodeEditorQuickEditRequest): Promise<string>
  /** Chord that opens the prompt on a selection, in the console's syntax. Default `Mod+I`. */
  shortcut?: string
  /** The model the card starts on (`provider::model`), e.g. the chat's;
      the card's picker can change it. */
  model?: string | null
  /** Label for the model when the console has no catalog to pick from. */
  modelLabel?: string | null
}

const QUICK_EDIT_MODEL_KEY = 'iii.quick-edit.model'
const QUICK_EDIT_EFFORT_KEY = 'iii.quick-edit.effort'

/** The last model and effort picked in a Quick Edit card, kept per browser. */
export function loadQuickEditModel(): {
  model: string | null
  thinkingLevel: string
} {
  try {
    return {
      model: localStorage.getItem(QUICK_EDIT_MODEL_KEY),
      thinkingLevel: localStorage.getItem(QUICK_EDIT_EFFORT_KEY) || 'default',
    }
  } catch {
    return { model: null, thinkingLevel: 'default' }
  }
}

export function saveQuickEditModel(model: string, thinkingLevel: string): void {
  try {
    localStorage.setItem(QUICK_EDIT_MODEL_KEY, model)
    localStorage.setItem(QUICK_EDIT_EFFORT_KEY, thinkingLevel)
  } catch {
    // Private mode: the pick lasts as long as the card.
  }
}

export const QUICK_EDIT_DEFAULT_SHORTCUT = 'Mod+I'

/** A chord as the caps a person reads (`⌘K`, `ctrl K`). */
export function shortcutLabel(binding: string): string {
  const caps = formatBinding(binding)
  return caps.every((cap) => cap.length === 1) ? caps.join('') : caps.join(' ')
}

/**
 * The lines of `after` that differ from `before`, as 0-based offsets into
 * `after` (inclusive), found by trimming the lines both share at the top
 * and at the bottom. Null when nothing changed or the change only removed
 * lines (there is nothing left to tint).
 */
export function changedLineSpan(
  before: string,
  after: string,
): { start: number; end: number } | null {
  if (before === after) return null
  const a = before.split('\n')
  const b = after.split('\n')
  let top = 0
  while (top < a.length && top < b.length && a[top] === b[top]) top += 1
  let bottom = 0
  while (
    bottom < a.length - top &&
    bottom < b.length - top &&
    a[a.length - 1 - bottom] === b[b.length - 1 - bottom]
  )
    bottom += 1
  const end = b.length - 1 - bottom
  return end < top ? null : { start: top, end }
}

/** Where `text` ends once inserted at 1-based `line`:`column`. */
export function insertedEnd(
  line: number,
  column: number,
  text: string,
): { line: number; column: number } {
  const lines = text.split('\n')
  if (lines.length === 1) return { line, column: column + text.length }
  return {
    line: line + lines.length - 1,
    column: (lines.at(-1)?.length ?? 0) + 1,
  }
}

export type QuickEditPhase = 'prompt' | 'running' | 'review'

const SCOPE_LABELS: Record<CodeEditorQuickEditScope, string> = {
  selection: 'Edit selection',
  file: 'Edit full file',
}

interface QuickEditCardProps {
  phase: QuickEditPhase
  scope: CodeEditorQuickEditScope
  onScopeChange: (scope: CodeEditorQuickEditScope) => void
  instruction: string
  onInstructionChange: (text: string) => void
  error: string | null
  modelLabel?: string | null
  /** The picked model; null until one is chosen or defaulted. */
  model: string | null
  /** The caller's starting model, used when nothing was picked before. */
  defaultModel?: string | null
  thinkingLevel: string
  onModelChange: (model: string, thinkingLevel: string) => void
  onSubmit: () => void
  onStop: () => void
  onClose: () => void
  onAccept: () => void
  onReject: () => void
  /** The card's own text box, so the editor can put the caret in it. */
  inputRef: React.RefObject<HTMLTextAreaElement | null>
  /** The card's root, focused in review so its keys still land here. */
  rootRef: React.RefObject<HTMLDivElement | null>
}

/** Rows the instruction box grows to before it scrolls. */
const MAX_ROWS = 6

/**
 * The card. Its keys are handled on the native event and stopped there:
 * the card lives inside Monaco's DOM, and an Enter or ⌘Z that bubbled on
 * would reach the editor's own keybindings and edit the code behind it.
 */
export function QuickEditCard({
  phase,
  scope,
  onScopeChange,
  instruction,
  onInstructionChange,
  error,
  modelLabel,
  model,
  defaultModel,
  thinkingLevel,
  onModelChange,
  onSubmit,
  onStop,
  onClose,
  onAccept,
  onReject,
  inputRef,
  rootRef,
}: QuickEditCardProps) {
  const latest = React.useRef({
    phase,
    instruction,
    onSubmit,
    onStop,
    onClose,
    onAccept,
    onReject,
  })
  latest.current = {
    phase,
    instruction,
    onSubmit,
    onStop,
    onClose,
    onAccept,
    onReject,
  }

  React.useEffect(() => {
    const node = rootRef.current
    if (!node) return
    const onKeyDown = (event: KeyboardEvent) => {
      const current = latest.current
      const mod = event.metaKey || event.ctrlKey
      if (event.isComposing) {
        event.stopPropagation()
        return
      }
      // A key typed into the scope menu (it portals elsewhere) never gets here.
      if (event.key === 'Escape') {
        event.preventDefault()
        if (current.phase === 'running') current.onStop()
        else if (current.phase === 'review') current.onReject()
        else current.onClose()
      } else if (current.phase === 'review' && event.key === 'Enter') {
        event.preventDefault()
        current.onAccept()
      } else if (
        current.phase === 'review' &&
        mod &&
        event.key === 'Backspace'
      ) {
        event.preventDefault()
        current.onReject()
      } else if (
        current.phase === 'prompt' &&
        event.key === 'Enter' &&
        !event.shiftKey
      ) {
        event.preventDefault()
        if (current.instruction.trim() !== '') current.onSubmit()
      }
      event.stopPropagation()
    }
    // Monaco listens for these too (chords, its own key-up handling).
    const stop = (event: Event) => event.stopPropagation()
    node.addEventListener('keydown', onKeyDown)
    node.addEventListener('keyup', stop)
    node.addEventListener('keypress', stop)
    return () => {
      node.removeEventListener('keydown', onKeyDown)
      node.removeEventListener('keyup', stop)
      node.removeEventListener('keypress', stop)
    }
  }, [rootRef])

  // Grow with the instruction, up to a handful of lines.
  React.useLayoutEffect(() => {
    const input = inputRef.current
    if (!input) return
    input.style.height = 'auto'
    const line = Number.parseFloat(getComputedStyle(input).lineHeight) || 20
    input.style.height = `${Math.min(input.scrollHeight, line * MAX_ROWS)}px`
  }, [instruction, inputRef])

  const running = phase === 'running'
  const canSend = instruction.trim() !== '' && phase === 'prompt'

  return (
    <div
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-label="Quick edit"
      data-quick-edit=""
      className={cn(
        NEUTRAL_SURFACE,
        'w-full rounded-xl bg-popover font-inter text-sm text-popover-foreground shadow-md ring-1 ring-foreground/10 outline-none',
      )}
    >
      <div className="flex items-start gap-2 px-3 pt-2.5">
        {phase === 'review' ? (
          <p
            className="min-w-0 flex-1 truncate py-0.5 text-ink-faint"
            title={instruction}
          >
            {instruction}
          </p>
        ) : (
          <textarea
            ref={inputRef}
            value={instruction}
            onChange={(event) => onInstructionChange(event.currentTarget.value)}
            placeholder={
              scope === 'file' ? 'Edit the whole file' : 'Edit selected code'
            }
            aria-label="Describe the edit"
            rows={1}
            readOnly={running}
            spellCheck={false}
            className="block min-h-6 min-w-0 flex-1 resize-none bg-transparent py-0.5 text-sm leading-5 text-ink outline-none placeholder:text-ink-faint read-only:text-ink-faint"
          />
        )}
        <Hint label="Close (Esc)">
          <button
            type="button"
            aria-label="Close quick edit"
            onClick={phase === 'review' ? onReject : running ? onStop : onClose}
            className="-mr-1 inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            <X aria-hidden className="size-4" />
          </button>
        </Hint>
      </div>
      {error ? (
        <p role="alert" className="px-3 pt-1 text-sm text-alert">
          {error}
        </p>
      ) : null}
      <div className="flex items-center gap-2 px-3 pt-1.5 pb-2.5">
        {phase === 'prompt' ? (
          <QuickEditModelPicker
            model={model}
            defaultModel={defaultModel}
            thinkingLevel={thinkingLevel}
            fallbackLabel={modelLabel}
            onChange={onModelChange}
          />
        ) : null}
        <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
          {running ? (
            <span className="inline-flex items-center gap-1.5">
              <LoaderCircle
                aria-hidden
                className="size-4 animate-spin motion-reduce:animate-none"
              />
              Editing
              {model
                ? ` with ${shortModel(model)}`
                : modelLabel
                  ? ` with ${modelLabel}`
                  : ''}
              …
            </span>
          ) : phase === 'review' ? (
            'Changed lines are highlighted. ⌘Z still undoes the edit.'
          ) : null}
        </span>
        {phase === 'review' ? (
          <>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={onReject}
            >
              Reject
              <span className="text-xs text-ink-faint">esc</span>
            </Button>
            <Button type="button" size="sm" onClick={onAccept}>
              Accept
              <span className="text-xs opacity-70">⏎</span>
            </Button>
          </>
        ) : (
          <>
            <DropdownMenu>
              <DropdownMenuTrigger
                disabled={running}
                className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-xs text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus disabled:pointer-events-none"
              >
                {SCOPE_LABELS[scope]}
                <ChevronDown aria-hidden className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className="min-w-[11rem]"
                finalFocus={inputRef}
              >
                <DropdownMenuRadioGroup
                  value={scope}
                  onValueChange={(value) => {
                    if (value === 'selection' || value === 'file')
                      onScopeChange(value)
                  }}
                >
                  <DropdownMenuRadioItem value="selection">
                    {SCOPE_LABELS.selection}
                  </DropdownMenuRadioItem>
                  <DropdownMenuRadioItem value="file">
                    {SCOPE_LABELS.file}
                  </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
            {running ? (
              <Hint label="Stop (Esc)">
                <button
                  type="button"
                  aria-label="Stop"
                  onClick={onStop}
                  className="inline-flex size-7 items-center justify-center rounded-full bg-surface-hover text-ink hover:bg-surface-selected focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                >
                  <Square aria-hidden className="size-4 fill-current" />
                </button>
              </Hint>
            ) : (
              <Hint label="Submit (⏎)">
                <button
                  type="button"
                  aria-label="Submit edit"
                  disabled={!canSend}
                  onClick={onSubmit}
                  className={cn(
                    'inline-flex size-7 items-center justify-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
                    canSend
                      ? 'bg-foreground text-background hover:opacity-90'
                      : 'bg-surface-hover text-ink-faint',
                  )}
                >
                  <ArrowUp aria-hidden className="size-4" />
                </button>
              </Hint>
            )}
          </>
        )}
      </div>
    </div>
  )
}

/** `provider::model` reads better without its provider. */
function shortModel(model: string): string {
  const at = model.indexOf('::')
  return at === -1 ? model : model.slice(at + 2)
}

/**
 * The chat's own model picker, over the console's catalog. It opens on the
 * last model picked here, else the caller's (the chat's), else the catalog's
 * first. Without a catalog (an embed outside the console) it shows the
 * caller's label instead.
 */
function QuickEditModelPicker({
  model,
  defaultModel,
  thinkingLevel,
  fallbackLabel,
  onChange,
}: {
  model: string | null
  defaultModel?: string | null
  thinkingLevel: string
  fallbackLabel?: string | null
  onChange: (model: string, thinkingLevel: string) => void
}) {
  const ctx = useConversationsCtxOptional()
  const options = ctx?.modelOptions ?? []
  const latest = React.useRef({ onChange, thinkingLevel })
  latest.current = { onChange, thinkingLevel }
  React.useEffect(() => {
    if (model !== null || options.length === 0) return
    const saved = loadQuickEditModel()
    const offered = (id: string | null | undefined): id is string =>
      !!id && options.some((option) => option.id === id)
    if (offered(saved.model))
      latest.current.onChange(saved.model, saved.thinkingLevel)
    else if (offered(defaultModel))
      latest.current.onChange(defaultModel, latest.current.thinkingLevel)
    else if (options[0])
      latest.current.onChange(options[0].id, latest.current.thinkingLevel)
  }, [model, options, defaultModel])
  if (!ctx) {
    return fallbackLabel ? (
      <span className="min-w-0 truncate text-xs text-ink-faint">
        {fallbackLabel}
      </span>
    ) : null
  }
  return (
    <ModelPicker
      value={model}
      options={options}
      thinkingLevel={thinkingLevel}
      onChange={(next, level) => {
        const effort = level ?? thinkingLevel
        saveQuickEditModel(next, effort)
        onChange(next, effort)
      }}
      onThinkingLevelChange={(level) => {
        if (model) saveQuickEditModel(model, level)
        if (model) onChange(model, level)
      }}
      loading={ctx.catalogLoading}
      showRefresh={false}
      triggerAppearance="subtle"
      className="min-w-0 max-w-[16rem]"
    />
  )
}
