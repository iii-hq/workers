type StepListener = (delta: number) => void

const listeners = new Set<StepListener>()

/** Move the active chat by whole turns; the mounted message list answers. */
export function stepChatTurn(delta: number): void {
  for (const listener of [...listeners]) listener(delta)
}

export function subscribeChatTurnStep(listener: StepListener): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
