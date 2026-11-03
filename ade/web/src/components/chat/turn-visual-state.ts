import { observerActivityPairKey } from '@/components/observer-activity/grouping'
import type { Message } from '@/types/chat'

export type TurnVisualPhase =
  | 'idle'
  | 'waiting'
  | 'reasoning'
  | 'calling'
  | 'answering'

export interface TurnVisualState {
  phase: TurnVisualPhase
  showWaiting: boolean
  /** The user input or observer activity that began the current turn. */
  turnKey?: string
  /** A visible result exists and the active turn is waiting for its next step. */
  betweenSteps: boolean
}

/**
 * Derive presentation from protocol truth without making motion a second
 * source of truth. The UI may retain the previous surface briefly for exit,
 * but this selector always describes the current transcript immediately.
 */
export function deriveTurnVisualState(
  messages: readonly Message[],
  working: boolean,
): TurnVisualState {
  let turnKey: string | undefined
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]
    // Observer notifications are model inputs too. Their durable entry id
    // resets the waiting clock for observer-only sessions just as a human
    // prompt does; the paired system record does not create a second reset.
    const observerPairKey = observerActivityPairKey(message)
    const isObserverRecord =
      message.role === 'system' &&
      message.kind === 'observer-fired' &&
      Boolean(message.observer)
    if (message.role === 'user' || isObserverRecord) {
      turnKey = observerPairKey ? `observer:${observerPairKey}` : message.id
      break
    }
  }

  if (!working) {
    return { phase: 'idle', showWaiting: false, turnKey, betweenSteps: false }
  }

  const last = messages[messages.length - 1]
  if (!last || last.role === 'user') {
    return { phase: 'waiting', showWaiting: true, turnKey, betweenSteps: false }
  }

  // A lifecycle record can precede the model-input notification. Treat that
  // first half as the start of the observer turn, not as progress left over
  // from the previous turn; its paired notification keeps the same turnKey.
  if (
    last.role === 'system' &&
    last.kind === 'observer-fired' &&
    last.observer
  ) {
    return {
      phase: 'waiting',
      showWaiting: true,
      turnKey,
      betweenSteps: false,
    }
  }

  if (last.role === 'thought') {
    return last.streaming
      ? {
          phase: 'reasoning',
          showWaiting: false,
          turnKey,
          betweenSteps: false,
        }
      : {
          phase: 'waiting',
          showWaiting: true,
          turnKey,
          betweenSteps: true,
        }
  }

  if (last.role === 'function-call') {
    if (last.running || last.pendingApproval) {
      return {
        phase: 'calling',
        showWaiting: false,
        turnKey,
        betweenSteps: false,
      }
    }
    return {
      phase: 'waiting',
      showWaiting: true,
      turnKey,
      betweenSteps: true,
    }
  }

  if (last.role === 'assistant') {
    if (!last.streaming && last.stopReason === 'function_call') {
      return {
        phase: 'waiting',
        showWaiting: true,
        turnKey,
        betweenSteps: true,
      }
    }
    return {
      phase: 'answering',
      showWaiting: false,
      turnKey,
      betweenSteps: false,
    }
  }

  return { phase: 'waiting', showWaiting: true, turnKey, betweenSteps: true }
}
