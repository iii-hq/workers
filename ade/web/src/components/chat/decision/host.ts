import { useSyncExternalStore } from 'react'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import type {
  DecisionChatHandle,
  DecisionChatRequest,
  Host,
} from '@/types/injectable-ui'
import { createDecisionSideChat, findDecisionSideChat } from './side-chat'

interface Caller {
  tabId: string
  paneId: string
}

interface DecisionChatWorkspace {
  caller(screen?: string): Caller
  open(sessionId: string, caller: Caller): void
  close(sessionId: string): void
}

interface Binding {
  copy: SessionMeta
  request: DecisionChatRequest
  editable: boolean
  closed: boolean
}

let workspace: DecisionChatWorkspace | undefined
const bindings = new Map<string, Binding>()
const listeners = new Set<() => void>()
const pendingBindings = new Map<string, symbol>()
const openingCopies = new Map<string, Promise<{ meta: SessionMeta }>>()
function emit() {
  for (const listener of listeners) listener()
}
function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function registerDecisionChatWorkspace(value: DecisionChatWorkspace) {
  workspace = value
  return () => {
    if (workspace === value) workspace = undefined
  }
}

export function useDecisionChatBinding(sessionId: string) {
  return useSyncExternalStore(subscribe, () => bindings.get(sessionId))
}

function bind(
  copy: SessionMeta,
  request: DecisionChatRequest,
  token: symbol,
): DecisionChatHandle {
  const id = copy.session_id
  let binding: Binding = { copy, request, editable: true, closed: false }
  if (pendingBindings.get(request.key) === token) {
    bindings.set(id, binding)
    emit()
  }
  return {
    update(state) {
      if (bindings.get(id) !== binding) return
      if (state.closed) pendingBindings.delete(request.key)
      binding = { ...binding, ...state }
      bindings.set(id, binding)
      emit()
    },
    detach() {
      if (bindings.get(id) !== binding) return
      bindings.delete(id)
      emit()
    },
    close() {
      if (bindings.get(id) === binding) workspace?.close(id)
    },
  }
}

/** Host capability exposed to injected pages that own decision inboxes. */
export function decisionChatCapabilities(
  iii: Pick<IiiClient, 'call'>,
  callerScreen?: string,
): Required<Pick<Host['chat'], 'openDecisionChat' | 'rebindDecisionChat'>> {
  return {
    async openDecisionChat(request) {
      const current = workspace
      if (!current) throw new Error('Decision chats require a workspace tab.')
      const caller = current.caller(callerScreen)
      const token = Symbol()
      pendingBindings.set(request.key, token)
      const preparation = createDecisionSideChat(iii, request)
      openingCopies.set(request.key, preparation)
      try {
        const { meta } = await preparation
        current.open(meta.session_id, caller)
        return bind(meta, request, token)
      } finally {
        if (pendingBindings.get(request.key) === token)
          pendingBindings.delete(request.key)
        if (openingCopies.get(request.key) === preparation)
          openingCopies.delete(request.key)
      }
    },
    async rebindDecisionChat(request) {
      const token = Symbol()
      pendingBindings.set(request.key, token)
      try {
        const opening = openingCopies.get(request.key)
        const copy = opening
          ? (await opening).meta
          : await findDecisionSideChat(iii, request.key)
        return copy ? bind(copy, request, token) : null
      } finally {
        if (pendingBindings.get(request.key) === token)
          pendingBindings.delete(request.key)
      }
    },
  }
}
