import { RefreshCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from './controls'

/**
 * "Scan again", with a glyph that turns at least once per click — a scan
 * that answers in 20ms still looks like it did something — and keeps
 * turning while the scan runs, stopping only at the end of a whole turn so
 * it never snaps back mid-rotation. The label stays put so the header
 * does not reflow.
 */
export function ScanButton({
  scanning,
  disabled,
  onScan,
}: {
  scanning: boolean
  disabled?: boolean
  onScan: () => void
}) {
  const [turning, setTurning] = useState(false)
  const scanningRef = useRef(scanning)
  scanningRef.current = scanning

  // A scan that started elsewhere (the inline "Scan again") turns it too.
  useEffect(() => {
    if (scanning) setTurning(true)
  }, [scanning])

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => {
        setTurning(true)
        onScan()
      }}
      disabled={disabled || scanning}
      aria-busy={scanning || undefined}
    >
      <RefreshCw
        aria-hidden
        className={
          turning ? 'animate-spin motion-reduce:animate-none' : undefined
        }
        onAnimationIteration={() => {
          if (!scanningRef.current) setTurning(false)
        }}
      />
      Scan again
    </Button>
  )
}
