import { CornerDownLeft, Maximize2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Dialog, DialogContent, DialogTitle } from '@/components/shadcn/dialog'
import { Hint } from '@/components/ui/Hint'
import { attachmentsFromFiles } from '@/lib/attachments/from-files'
import { recentHarnessProjectPaths } from '@/lib/backend/projects'
import { formatBinding } from '@/lib/keybindings/bindings'
import { isSendEvent, sendBinding, useSendKey } from '@/lib/send-key'
import { loadLastModel, loadLastThinkingLevel } from '@/lib/storage'
import { useExtComposerActions } from '@/lib/ui-slots'
import {
  type AgentProfileChangeOptions,
  type AgentProfileSnapshot,
  type Attachment,
  DEFAULT_THINKING_LEVEL,
  type ModelId,
  type ModelOption,
  type ThinkingLevel,
} from '@/types/chat'
import { AgentProfilePicker } from './AgentProfilePicker'
import { AttachmentButton } from './AttachmentButton'
import { AttachmentChip } from './AttachmentChip'
import { toolbarIconButtonClass } from './composer-chrome'
import { DirectoryPicker } from './DirectoryPicker'
import { ModelPicker } from './ModelPicker'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SystemPromptState,
} from './system-prompt-selection'

export interface NewChatDraft {
  text: string
  model: ModelId | null
  thinkingLevel: ThinkingLevel
  workingDir: string | null
  /** The Directory agent profile to freeze onto the chat, if one is chosen. */
  agentProfile?: AgentProfileSnapshot
  systemPrompt: SystemPromptState
  /** Files attached in the dialog; they go out with the first message. */
  attachments: Attachment[]
}

interface NewChatDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  modelOptions: ModelOption[]
  catalogLoading: boolean
  /** The user pressed Create: start the chat and send this first message. */
  onCreate: (draft: NewChatDraft) => void
  /** Carry the dialog's choices into a blank chat, unsent, to finish there. */
  onOpenFull?: (draft: NewChatDraft) => void
}

/** The id an injected composer action (the voice worker's mic) is handed
 *  before the chat exists; dictation types into the focused field. */
const DIALOG_ACTION_SESSION = 'new-chat-dialog'

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
 * Every row is a control the chat already has (the directory picker, the
 * agent profile picker and the model picker from the composer), so a choice
 * made here is the same choice the chat would have offered one step later.
 */
export function NewChatDialog({
  open,
  onOpenChange,
  modelOptions,
  catalogLoading,
  onCreate,
  onOpenFull,
}: NewChatDialogProps) {
  const [text, setText] = useState('')
  const sendKey = useSendKey()
  // The send key's modifier caps (⌘ or ⇧), shown ahead of the Enter cap.
  const sendModifiers = formatBinding(sendBinding(sendKey)).slice(0, -1)
  const [attachments, setAttachments] = useState<Attachment[]>([])
  const [model, setModel] = useState<ModelId | null>(null)
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>(
    DEFAULT_THINKING_LEVEL,
  )
  const [workingDir, setWorkingDir] = useState<string | null>(null)
  const [agentProfile, setAgentProfile] = useState<AgentProfileSnapshot>()
  const [systemPrompt, setSystemPrompt] = useState<SystemPromptState>(
    DEFAULT_SYSTEM_PROMPT_STATE,
  )
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Each opening starts from the user's last choices, not from the
  // previous dialog's leftovers.
  useEffect(() => {
    if (!open) {
      // Cleared on close, not on open: the picker's own default pick lands
      // while the dialog opens and must not be wiped behind it.
      setAgentProfile(undefined)
      setSystemPrompt(DEFAULT_SYSTEM_PROMPT_STATE)
      return
    }
    setText('')
    setAttachments([])
    setModel(startingModel(modelOptions))
    setThinkingLevel(loadLastThinkingLevel() ?? DEFAULT_THINKING_LEVEL)
    setWorkingDir(recentHarnessProjectPaths()[0] ?? null)
  }, [open, modelOptions])

  // A profile brings its model and reasoning effort, as it does in the
  // composer; the pickers below still have the last word.
  const changeAgentProfile = (
    next: AgentProfileSnapshot | undefined,
    options?: AgentProfileChangeOptions,
  ) => {
    setAgentProfile(next)
    if (!next) return
    if (next.model) setModel(next.model)
    if (!options?.keepThinkingLevel || next.reasoningEffort) {
      setThinkingLevel(next.reasoningEffort ?? DEFAULT_THINKING_LEVEL)
    }
  }

  // A catalogue that arrives after the dialog opened fills an empty pick.
  useEffect(() => {
    if (open && model === null && modelOptions.length > 0) {
      setModel(startingModel(modelOptions))
    }
  }, [open, model, modelOptions])

  // The voice worker's mic (and any other injected composer action), the
  // same component the composer's toolbar mounts. Dictation types into the
  // field that had focus, so it lands in the prompt below.
  const extActions = useExtComposerActions()
  const actions = useMemo(
    () =>
      [...extActions]
        .sort((a, b) => a.id.localeCompare(b.id))
        .map(({ id, render: Action }) => (
          <Action
            key={id}
            sessionId={DIALOG_ACTION_SESSION}
            isStreaming={false}
          />
        )),
    [extActions],
  )

  const attach = (next: Attachment[]) => {
    if (next.length > 0) setAttachments((current) => [...current, ...next])
    textareaRef.current?.focus()
  }

  const draft = (): NewChatDraft => ({
    text: text.trim(),
    model,
    thinkingLevel,
    workingDir,
    agentProfile,
    systemPrompt,
    attachments,
  })

  const ready = text.trim().length > 0
  const create = () => {
    if (!ready) return
    onCreate(draft())
    onOpenChange(false)
  }
  const openFull = () => {
    onOpenFull?.(draft())
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby={undefined}
        className="flex w-[min(56rem,calc(100vw-2rem))] max-w-none [--dialog-max-width:none] flex-col gap-0 overflow-visible bg-dialog-surface p-0 font-inter text-neutral-900 ring-1 ring-dialog-edge dark:text-neutral-50"
        initialFocus={textareaRef}
      >
        <DialogTitle className="sr-only">New chat</DialogTitle>

        {/* Where: the project the chat works in, as a dropdown chip in the
            corner. The caret and the tooltip say it can be changed; the
            path lives in the tooltip so the chip stays one word wide. */}
        <div className="flex h-12 items-center gap-2 border-b border-neutral-100 pr-12 pl-2 dark:border-neutral-800">
          <DirectoryPicker
            value={workingDir}
            onChange={setWorkingDir}
            triggerAppearance="chip"
            emptyLabel="Choose project"
            popoverAlign="start"
            className="max-w-[min(20rem,60vw)]"
          />
          {/* The way out to the full composer, beside the close button:
              what is typed and chosen here comes along, unsent. */}
          {onOpenFull ? (
            <Hint label="Open as a full chat" side="bottom">
              <button
                type="button"
                onClick={openFull}
                aria-label="Open as a full chat"
                className={`ml-auto ${toolbarIconButtonClass}`}
              >
                <Maximize2 className="size-4" aria-hidden />
              </button>
            </Hint>
          ) : null}
        </div>

        {/* What: the first message. The Settings send key sends (Enter by
            default); every other Enter breaks a line natively. */}
        <textarea
          ref={textareaRef}
          name="new-chat-prompt"
          aria-label="What do you want to work on?"
          placeholder="What do you want to work on?"
          value={text}
          onChange={(event) => setText(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return
            if (isSendEvent(sendKey, event)) {
              event.preventDefault()
              create()
            }
          }}
          // A pasted screenshot or file attaches, as it does in the composer.
          onPaste={async (event) => {
            const files = Array.from(event.clipboardData.files)
            if (files.length === 0) return
            event.preventDefault()
            attach(await attachmentsFromFiles(files))
          }}
          rows={6}
          className="min-h-44 w-full resize-none bg-transparent px-4 py-3 text-[14px] leading-6 text-neutral-900 outline-none placeholder:text-neutral-400 dark:text-neutral-50 dark:placeholder:text-neutral-500"
        />

        {attachments.length > 0 ? (
          <ul
            aria-label="Attachments"
            className="flex flex-wrap gap-1.5 px-4 pb-2"
          >
            {attachments.map((attachment) => (
              <li key={attachment.id}>
                <AttachmentChip
                  attachment={attachment}
                  appearance="composer"
                  onRemove={(id) =>
                    setAttachments((current) =>
                      current.filter((item) => item.id !== id),
                    )
                  }
                />
              </li>
            ))}
          </ul>
        ) : null}

        {/* How: the model, then the agent profile (the composer's order);
            then the composer's own tools — the mic, attach — and the one
            action. The row is the recording bar's containing block. */}
        <div className="new-chat-dialog-toolbar relative flex h-14 items-center justify-between gap-3 px-3">
          <div className="flex min-w-0 items-center gap-1">
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
            <AgentProfilePicker
              systemPrompt={systemPrompt}
              onSystemPromptChange={setSystemPrompt}
              agentProfile={agentProfile}
              onAgentProfileChange={changeAgentProfile}
              preselectDefaultAgent
            />
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {actions}
            <AttachmentButton
              onAttach={attach}
              className={toolbarIconButtonClass}
            />
            <Button
              type="button"
              onClick={create}
              disabled={!ready}
              className="ml-1 pr-1.5"
            >
              Create
              {/* The send-key cap rides on the primary fill in its own ink. */}
              <span
                aria-hidden
                className="inline-flex h-5 min-w-5 items-center justify-center gap-0.5 rounded-md bg-primary-foreground/15 px-0.5 text-[12px] text-primary-foreground/80"
              >
                {sendModifiers.map((cap) => (
                  <span key={cap}>{cap}</span>
                ))}
                <CornerDownLeft className="size-4" />
              </span>
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
