// "Fill with Jev": one `eval::propose-validation` call at a time, its answer
// kept as a `JevFill`. The call only proposes; the dialog puts the pair on its
// pickers through `onPair`, and attaching stays the user's. Closing the dialog
// discards a late answer.
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import { choosePair, classifyFillFailure, fillFromResponse, type JevFill, type JevPair, settlePicked } from './jev-fill'
import type { Side } from './validation-lookup'

interface Options {
  api: EvalApi
  evaluationId: string
  suggestionIndex: number
  /** Jev's pair, or the one the user switched to: put it on both pickers. */
  onPair: (pair: JevPair) => void
  /** Jev gave no pair: empty the pickers that still held its earlier one, so no stale pair stays unlabelled. */
  onClear: (sides: Record<Side, boolean>) => void
  /** The E2E service did not list runs for Jev: the run list is down as well. */
  onE2eDown: () => void
}

export function useJevFill({ api, evaluationId, suggestionIndex, onPair, onClear, onE2eDown }: Options) {
  const [fill, setFill] = useState<JevFill>({ kind: 'idle' })
  const latest = useRef(0)
  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])

  const ask = useCallback(() => {
    const request = ++latest.current
    const current = (): boolean => alive.current && request === latest.current
    // The pickers are locked while asking, so what Jev held now is what an answer without a pair replaces.
    const held = fill.kind === 'proposed' ? fill.marks : undefined
    setFill({ kind: 'asking' })
    api
      .proposeValidation(evaluationId, suggestionIndex)
      .then((response) => {
        if (!current()) return
        const next = fillFromResponse(response)
        setFill(next)
        if (next.kind === 'proposed') onPair(next.pairs[0])
        else if (held) onClear(held)
      })
      .catch((error) => {
        if (!current()) return
        const failure = classifyFillFailure(errorMessage(error))
        if (failure.kind === 'e2e_unavailable') {
          setFill({ kind: 'idle' })
          onE2eDown()
        } else {
          setFill({ kind: 'failed', message: failure.message })
        }
        if (held) onClear(held)
      })
  }, [api, evaluationId, suggestionIndex, fill, onPair, onClear, onE2eDown])

  /** Switches to another of Jev's pairs: both pickers are Jev's again. */
  const choose = (index: number) => {
    if (fill.kind !== 'proposed') return
    setFill(choosePair(fill, index))
    onPair(fill.pairs[index])
  }

  /** The user picked a run: marks follow the ids the pickers now hold. */
  const picked = (pair: Pick<JevPair, 'baseline' | 'candidate'>) => setFill((last) => settlePicked(last, pair))

  return { fill, ask, choose, picked }
}
