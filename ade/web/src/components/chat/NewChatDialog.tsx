import { CornerDownLeft } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Dialog, DialogContent, DialogTitle } from '@/components/shadcn/dialog'
import { recentHarnessProjectPaths } from '@/lib/backend/projects'
import { loadLastModel, loadLastThinkingLevel } from '@/lib/storage'
import { cn } from '@/lib/utils'
import {
  DEFAULT_THINKING_LEVEL,
  type ModelId,
  type ModelOption,
  type ThinkingLevel,
} from '@/types/chat'
import { DirectoryPicker } from './DirectoryPicker'
import { ModelPicker } from './ModelPicker'

export interface NewChatDraft {
  text: string
  model: ModelId | null
  thinkingLevel: ThinkingLevel
  workingDir: string | null
}

interface NewChatDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  modelOptions: ModelOption[]
  catalogLoading: boolean
  /** The user pressed Create: start the chat and send this first message. */
  onCreate: (draft: NewChatDraft) => void
}

/** The model the dialog opens on: the last one used when it is still
 *  offered, else the first in the catalogue. */
function startingModel(options: ModelOption[]): ModelId | null {
  const last = loadLastModel()
  if (last && options.some((option) => option.id === last)) return last
  return options[0]?.id ?? null
}

/**
 * New chat as one decision, Conductor-style: pick the project, say what to
 * work on, pick the model, press Create. The chat is created with the first
 * message already on its way, so the composer never shows an empty draft.
 *
 * Every row is a control the chat already has (the directory picker and the
 * model picker from the composer), so a choice made here is the same choice
 * the chat would have offered one step later.
 */
export function NewChatDialog({
  open,
  onOpenChange,
  modelOptions,
  catalogLoading,
  onCreate,
}: NewChatDialogProps) {
  const [text, setText] = useState('')
  const [model, setModel] = useState<ModelId | null>(null)
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>(
    DEFAULT_THINKING_LEVEL,
  )
  const [workingDir, setWorkingDir] = useState<string | null>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Each opening starts from the user's last choices, not from the
  // previous dialog's leftovers.
  useEffect(() => {
    if (!open) return
    setText('')
    setModel(startingModel(modelOptions))
    setThinkingLevel(loadLastThinkingLevel() ?? DEFAULT_THINKING_LEVEL)
    setWorkingDir(recentHarnessProjectPaths()[0] ?? null)
  }, [open, modelOptions])

  // A catalogue that arrives after the dialog opened fills an empty pick.
  useEffect(() => {
    if (open && model === null && modelOptions.length > 0) {
      setModel(startingModel(modelOptions))
    }
  }, [open, model, modelOptions])

  const ready = text.trim().length > 0
  const create = () => {
    if (!ready) return
    onCreate({ text: text.trim(), model, thinkingLevel, workingDir })
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby={undefined}
        className="flex w-[min(56rem,calc(100vw-2rem))] max-w-none [--dialog-max-width:none] flex-col gap-0 overflow-visible bg-white p-0 font-inter text-neutral-900 ring-1 ring-neutral-200 dark:bg-neutral-900 dark:text-neutral-50 dark:ring-neutral-800"
        initialFocus={textareaRef}
      >
        <DialogTitle className="sr-only">New chat</DialogTitle>

        {/* Where: the project the chat works in, as a dropdown chip in the
            corner. The caret and the tooltip say it can be changed; the
            path lives in the tooltip so the chip stays one word wide. */}
        <div className="flex h-12 items-center gap-2 border-b border-neutral-200 pr-14 pl-2 dark:border-neutral-800">
          <DirectoryPicker
            value={workingDir}
            onChange={setWorkingDir}
            triggerAppearance="chip"
            emptyLabel="Choose project"
            popoverAlign="start"
            className="max-w-[min(20rem,60vw)]"
          />
        </div>

        {/* What: the first message. Enter sends, Shift+Enter breaks a line. */}
        <textarea
          ref={textareaRef}
          name="new-chat-prompt"
          aria-label="What do you want to work on?"
          placeholder="What do you want to work on?"
          value={text}
          onChange={(event) => setText(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              create()
            }
          }}
          rows={6}
          className="min-h-44 w-full resize-none bg-transparent px-4 py-3 text-[14px] leading-6 text-neutral-900 outline-none placeholder:text-neutral-400 dark:text-neutral-50 dark:placeholder:text-neutral-500"
        />

        {/* How: the model, and the one action. */}
        <div className="flex h-14 items-center justify-between gap-3 px-3">
          <ModelPicker
            value={model}
            options={modelOptions}
            thinkingLevel={thinkingLevel}
            onChange={(next, level) => {
              setModel(next)
              if (level !== undefined) setThinkingLevel(level)
            }}
            onThinkingLevelChange={setThinkingLevel}
            loading={catalogLoading}
            showRefresh={false}
            triggerAppearance="subtle"
            className="min-w-0 max-w-[18rem]"
          />
          <button
            type="button"
            onClick={create}
            disabled={!ready}
            className={cn(
              'inline-flex h-8 shrink-0 items-center gap-2 rounded-xl pr-2 pl-3 text-[13px] font-medium transition-colors duration-[var(--motion-duration-control)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 disabled:cursor-not-allowed dark:focus-visible:ring-neutral-400',
              ready
                ? 'bg-neutral-900 text-neutral-50 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200'
                : 'bg-neutral-100 text-neutral-400 dark:bg-neutral-800 dark:text-neutral-500',
            )}
          >
            Create
            <span
              aria-hidden
              className={cn(
                'inline-flex size-5 items-center justify-center rounded-md',
                ready
                  ? 'bg-neutral-800 text-neutral-300 dark:bg-neutral-200 dark:text-neutral-600'
                  : 'bg-neutral-200/70 text-neutral-400 dark:bg-neutral-700 dark:text-neutral-500',
              )}
            >
              <CornerDownLeft className="size-4" />
            </span>
          </button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
