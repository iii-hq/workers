import { useCallback, useEffect, useMemo, useState } from 'react'
import { useConversationsCtx } from '@/lib/conversations-context'
import {
  type ModelLibraryRequest,
  onModelLibraryRequest,
} from '@/lib/model-library/open'
import {
  type PinnedModel,
  resolvePinnedModels,
  savePinnedModels,
  usePinnedModels,
} from '@/lib/pinned-models'
import {
  loadLastModel,
  saveLastModel,
  saveLastThinkingLevel,
} from '@/lib/storage'
import { type LibraryView, ModelLibraryDialog } from './ModelLibraryDialog'

/**
 * The Models dialog for whoever asks without a model picker of their own
 * (⌘K's provider rows): the same catalog, providers and pins the composer's
 * picker edits, opened on the page the request names.
 */
export function ModelLibraryHost() {
  const { modelOptions, presentProviders } = useConversationsCtx()
  const [request, setRequest] = useState<ModelLibraryRequest | null>(null)
  useEffect(() => onModelLibraryRequest(setRequest), [])

  const storedPinned = usePinnedModels()
  // Nothing pinned yet: the default list leads with the model new chats
  // start on, the same list the composer's picker shows.
  const open = request !== null
  const pinned = useMemo(
    () =>
      resolvePinnedModels(
        storedPinned,
        modelOptions,
        open ? loadLastModel() : null,
      ),
    [storedPinned, modelOptions, open],
  )
  // As in the picker: a new model in the Default slot is the person choosing
  // it, so the next chat starts there.
  const savePinned = useCallback(
    (list: PinnedModel[] | null) => {
      savePinnedModels(list)
      const first = list?.[0]
      if (first && first.id !== pinned[0]?.id) {
        saveLastModel(first.id)
        saveLastThinkingLevel(first.effort)
      }
    },
    [pinned],
  )

  const initialView = useMemo<LibraryView | undefined>(() => {
    if (request?.view === 'provider') {
      return { kind: 'provider', id: request.providerId }
    }
    if (request?.view === 'add-provider') return { kind: 'add-provider' }
    return undefined
  }, [request])

  return (
    <ModelLibraryDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setRequest(null)
      }}
      value={null}
      options={modelOptions}
      providers={presentProviders}
      pinned={pinned}
      thinkingLevel="default"
      onPinnedChange={savePinned}
      initialView={initialView}
      initialQuery={request?.view === 'library' ? request.query : undefined}
    />
  )
}
