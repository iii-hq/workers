/* A refresh icon that spins while its reload is in flight, and for at least
   one turn: a fast re-read still shows the click landed. */

import { useEffect, useRef, useState } from 'react'

/** One turn of `uiClasses.spin` (1 s linear). */
const MIN_SPIN_MS = 1000

export function useSpin(busy: boolean, minMs = MIN_SPIN_MS): boolean {
  const [spinning, setSpinning] = useState(busy)
  const startedRef = useRef(0)
  useEffect(() => {
    if (busy) {
      startedRef.current = Date.now()
      setSpinning(true)
      return
    }
    const left = minMs - (Date.now() - startedRef.current)
    if (left <= 0) {
      setSpinning(false)
      return
    }
    const timer = setTimeout(() => setSpinning(false), left)
    return () => clearTimeout(timer)
  }, [busy, minMs])
  return spinning
}
