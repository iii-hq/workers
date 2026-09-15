import { useEffect } from 'react'
import { acquireScreenWakeLock } from '@/lib/screen-wake-lock'

/** Hold one shared screen lease while active; release on idle or unmount. */
export function useScreenWakeLock(active: boolean): void {
  useEffect(() => {
    if (active) return acquireScreenWakeLock()
  }, [active])
}
