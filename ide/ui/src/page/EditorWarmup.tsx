/* Pull the code editor's chunk before the first file opens.

   The console loads Monaco (a few megabytes) the first time a CodeEditor
   mounts, so the first file a reader clicks used to open onto a blank
   pane for a second or two. The IDE has nothing else to show while the
   launcher is up, so once the page is idle this mounts one hidden, empty
   editor for a moment: the chunk downloads and is parsed, Monaco's
   workers start, and the real editor later finds it all in memory. The
   stand-in unmounts after a few seconds; the module cache keeps the work. */

import { CodeEditor } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'

const WARMUP_IDLE_TIMEOUT_MS = 1500
const WARMUP_HOLD_MS = 5000

export function EditorWarmup() {
  const [mounted, setMounted] = useState(false)
  useEffect(() => {
    let hold: number | null = null
    const start = () => {
      setMounted(true)
      hold = window.setTimeout(() => setMounted(false), WARMUP_HOLD_MS)
    }
    const idle =
      typeof window.requestIdleCallback === 'function'
        ? window.requestIdleCallback(start, { timeout: WARMUP_IDLE_TIMEOUT_MS })
        : window.setTimeout(start, WARMUP_IDLE_TIMEOUT_MS)
    return () => {
      if (typeof window.cancelIdleCallback === 'function') window.cancelIdleCallback(idle)
      else window.clearTimeout(idle)
      if (hold !== null) window.clearTimeout(hold)
    }
  }, [])
  if (!mounted) return null
  return (
    <div className="shui-editor-warmup" aria-hidden="true" inert>
      <CodeEditor value="" language="plaintext" readOnly onChange={() => {}} />
    </div>
  )
}
