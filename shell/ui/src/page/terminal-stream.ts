export interface TerminalFrame {
  sequence: number
  data: string
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

function assertAfterSequence(afterSequence: number): void {
  if (!isNonNegativeSafeInteger(afterSequence)) {
    throw new Error('invalid afterSequence')
  }
}

function assertFrameSequence(sequence: number): void {
  if (!isNonNegativeSafeInteger(sequence)) {
    throw new Error('invalid frame sequence')
  }
}

function framesEqual(left: TerminalFrame, right: TerminalFrame): boolean {
  return left.sequence === right.sequence && left.data === right.data
}

export function bufferTerminalFrame(
  pending: Map<number, TerminalFrame>,
  frame: TerminalFrame,
  afterSequence: number,
): TerminalFrame[] {
  assertAfterSequence(afterSequence)
  assertFrameSequence(frame.sequence)
  if (frame.sequence <= afterSequence) return []
  const existing = pending.get(frame.sequence)
  if (existing && !framesEqual(existing, frame)) {
    throw new Error(
      `conflicting terminal frame data for sequence ${frame.sequence}`,
    )
  }
  pending.set(frame.sequence, frame)
  const contiguous: TerminalFrame[] = []
  let sequence = afterSequence + 1
  while (pending.has(sequence)) {
    const next = pending.get(sequence)
    pending.delete(sequence)
    if (next) contiguous.push(next)
    sequence += 1
  }
  return contiguous
}

export function mergeTerminalFrames(
  replay: TerminalFrame[],
  pending: TerminalFrame[],
  afterSequence: number,
): TerminalFrame[] {
  assertAfterSequence(afterSequence)

  const merged = new Map<number, TerminalFrame>()

  for (const frame of [...replay, ...pending]) {
    assertFrameSequence(frame.sequence)
    if (frame.sequence <= afterSequence) continue
    const existing = merged.get(frame.sequence)
    if (existing) {
      if (!framesEqual(existing, frame)) {
        throw new Error(
          `conflicting terminal frame data for sequence ${frame.sequence}`,
        )
      }
      continue
    }
    merged.set(frame.sequence, frame)
  }

  return [...merged.values()].sort(
    (left, right) => left.sequence - right.sequence,
  )
}
