import {
  type ReactNode,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'

export function HoverTip({
  label,
  children,
}: {
  label: string
  children: ReactNode
}) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)
  const bubbleRef = useRef<HTMLSpanElement>(null)
  const triggerCenterRef = useRef(0)

  const hide = useCallback(() => setPos(null), [])
  const show = useCallback(
    (event: { currentTarget: EventTarget & Element }) => {
      const rect = event.currentTarget.getBoundingClientRect()
      triggerCenterRef.current = rect.left + rect.width / 2
      setPos({
        top: rect.bottom + 6,
        left: Math.max(8, rect.left),
      })
    },
    [],
  )

  useLayoutEffect(() => {
    if (!pos || !bubbleRef.current) return
    const width = bubbleRef.current.getBoundingClientRect().width
    const maxLeft = Math.max(8, window.innerWidth - width - 8)
    const left = Math.min(
      Math.max(8, triggerCenterRef.current - width / 2),
      maxLeft,
    )
    if (Math.abs(left - pos.left) > 0.5) {
      setPos((current) => (current ? { ...current, left } : current))
    }
  }, [pos])

  return (
    <fieldset
      className="shui-hover-tip"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {pos
        ? createPortal(
            <span
              ref={bubbleRef}
              className="shui-hover-tip-bubble"
              role="tooltip"
              style={{ top: pos.top, left: pos.left }}
            >
              {label}
            </span>,
            document.body,
          )
        : null}
    </fieldset>
  )
}
