import { createContext } from 'react'
import type { Attachment } from '@/types/chat'

/**
 * What the composer knows about its attachments, for the chips in the
 * editor. An `AttachmentNode` stores only the chip's id; the name, bytes
 * and thumbnail come from here, so a chip picks up the server id and the
 * hydrated bytes the moment the composer learns them. The composer owns
 * WHICH attachments there are (see `Composer`); a node whose id is not
 * here is a stray and is removed.
 */
export interface ComposerAttachmentsValue {
  byId: ReadonlyMap<string, Attachment>
}

export const ComposerAttachmentsContext =
  createContext<ComposerAttachmentsValue>({ byId: new Map() })
