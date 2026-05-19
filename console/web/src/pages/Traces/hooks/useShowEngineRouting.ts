// Shared persisted toggle for engine routing visibility, used by both
// the waterfall and flame-graph views. The state lives in
// localStorage so swapping views via the ViewSwitcher keeps the
// user's preference. Default is `false` (engine routing hidden).

import { useEffect, useState } from 'react'

const STORAGE_KEY = 'iii-trace-show-engine-routing'

export function useShowEngineRouting() {
  const [show, setShow] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false
    return window.localStorage.getItem(STORAGE_KEY) === '1'
  })

  useEffect(() => {
    window.localStorage.setItem(STORAGE_KEY, show ? '1' : '0')
  }, [show])

  return [show, setShow] as const
}
