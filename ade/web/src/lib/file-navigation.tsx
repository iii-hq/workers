import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useEffect,
} from 'react'
import { useConfirm } from '@/components/ui/ConfirmDialog'
import type { Message } from '@/types/chat'
import type { FileMentionRef } from './file-mention-token'
import { requestPanelOpen } from './panel-context'
import { getExtPage } from './ui-slots'

// A newer intent in ANY chat supersedes an older confirmation.
let latestFileIntent = 0

export const FileNavigationContext = createContext<
  ((ref: FileMentionRef, messageId?: string) => void | Promise<void>) | undefined
>(undefined)
export const FileMessageContext = createContext<string | undefined>(undefined)

export interface FileDirectory {
  path: string | null
  recorded: boolean
}

/** Scope markers are durable transcript facts, unlike the current chat folder.
 * A paged transcript can recover its first segment from previousPath. Missing
 * provenance is kept explicit and requires confirmation, not silent rebasing. */
export function messageFileDirectories(
  messages: readonly Message[],
  currentDir: string | null,
): Map<string, FileDirectory> {
  const first = messages.find(
    (m) => m.role === 'system' && m.kind === 'working-dir' && m.scope,
  )
  let directory: FileDirectory =
    first?.role === 'system' && first.scope
      ? { path: first.scope.previousPath ?? null, recorded: true }
      : { path: currentDir, recorded: false }
  const result = new Map<string, FileDirectory>()
  for (const message of messages) {
    if (message.role === 'system' && message.kind === 'working-dir' && message.scope) {
      directory = { path: message.scope.path, recorded: true }
    }
    result.set(message.id, directory)
  }
  return result
}

export function resolveChatFile(ref: FileMentionRef, workingDir: string | null): string {
  if (
    !ref.path ||
    ref.path.endsWith('/') ||
    ref.path.includes('\\') ||
    ref.path.startsWith('//') ||
    /^[a-z][a-z\d+.-]*:/i.test(ref.path) ||
    Array.from(ref.path).some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)
  ) {
    throw new Error('This is not a supported local file path. Use an absolute Unix path or a workspace-relative file.')
  }
  if (
    (ref.range && (!Number.isSafeInteger(ref.range.from) || !Number.isSafeInteger(ref.range.to) || ref.range.from < 1 || ref.range.to < ref.range.from)) ||
    (ref.column !== undefined && (!Number.isSafeInteger(ref.column) || ref.column < 1))
  ) {
    throw new Error('The file reference has an invalid line range or column.')
  }
  if (ref.path.startsWith('/')) return ref.path
  if (!workingDir?.startsWith('/')) {
    throw new Error('The original folder for this reference is unavailable. Use an absolute file path or select the correct conversation folder and try again.')
  }
  // Do not collapse .. client-side: symlink traversal belongs to the worker,
  // which canonicalizes and enforces its existing filesystem permissions.
  return `${workingDir.replace(/\/+$/, '')}/${ref.path}`
}

/** The composer and transcript share the same IDE entry point. */
export function openChatFile(ref: FileMentionRef, workingDir: string | null) {
  latestFileIntent += 1
  const path = resolveChatFile(ref, workingDir)
  requestPanelOpen({
    pageId: 'ide',
    context: {
      type: 'file',
      path,
      ...(ref.range ? { line: ref.range.from, endLine: ref.range.to } : {}),
      ...(ref.column ? { column: ref.column } : {}),
    },
  })
}

export function ChatFileNavigation({
  workingDir,
  enabled,
  messages,
  children,
}: {
  workingDir: string | null
  enabled: boolean
  messages?: readonly Message[]
  children: ReactNode
}) {
  const { confirm, dialog } = useConfirm()
  const requestSeq = useRef(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: a changed scope cancels the old confirmation
  useEffect(() => {
    requestSeq.current += 1
    return () => { requestSeq.current += 1 }
  }, [workingDir, enabled])
  const directories = useMemo(
    () => messageFileDirectories(messages ?? [], workingDir),
    [messages, workingDir],
  )
  const open = useCallback(
    async (ref: FileMentionRef, messageId?: string) => {
      const sequence = ++requestSeq.current
      const intent = ++latestFileIntent
      if (!enabled) throw new Error('File navigation is disabled for this conversation.')
      if (!getExtPage('ide')) {
        throw new Error('The IDE is not available. Enable or reconnect its worker, then click this reference again.')
      }
      const directory = messageId && messages
        ? directories.get(messageId) ?? { path: null, recorded: true }
        : { path: workingDir, recorded: messages === undefined }
      const path = resolveChatFile(ref, directory.path)
      if (!ref.path.startsWith('/') && !directory.recorded) {
        const accepted = await confirm({
          title: 'Confirm the folder for this reference',
          description: `This message has no recorded working folder. Open ${path} using the conversation's current folder? The file is the current disk version, not a historical snapshot.`,
          confirmLabel: 'Open file',
        })
        if (!accepted || sequence !== requestSeq.current || intent !== latestFileIntent) return
        if (!getExtPage('ide')) throw new Error('The IDE disconnected. Reconnect its worker and try again.')
      }
      openChatFile(ref, directory.path)
    },
    [workingDir, enabled, directories, messages, confirm],
  )
  return (
    <FileNavigationContext.Provider value={open}>
      {children}
      {dialog}
    </FileNavigationContext.Provider>
  )
}

/** Keep the exact originating message even through nested Markdown renders. */
export function useOpenMessageFile() {
  const open = useContext(FileNavigationContext)
  const messageId = useContext(FileMessageContext)
  return open ? (ref: FileMentionRef) => open(ref, messageId) : undefined
}
