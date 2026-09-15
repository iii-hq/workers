import { useEffect } from 'react'
import { acquireScreenWakeLock } from '@/lib/screen-wake-lock'

export function useScreenWakeLock(active: boolean): void {
  useEffect(() => {
    if (active) return acquireScreenWakeLock()
  }, [active])
}
