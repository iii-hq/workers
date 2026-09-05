import { createContext } from 'react'
import type { FileMentionRef } from '@/lib/file-mention-token'

/**
 * What the composer lets a mention pill do beyond sitting in the text.
 * Provided by LexicalShell above the editor; the decorator pills render
 * through a portal from inside it, so they see the same value.
 */
export interface ComposerMentionActions {
  /**
   * Open a mentioned file where it can be read — the shell explorer, on
   * the referenced lines. Absent when no surface can show files (mock
   * backend, shell worker away): the pill then only selects on click.
   */
  openFile?: (ref: FileMentionRef) => void
}

export const ComposerMentionContext = createContext<ComposerMentionActions>({})
