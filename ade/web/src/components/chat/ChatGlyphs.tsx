import { motion, useReducedMotion } from 'motion/react'

/** Draw only on a state change, so opening historical messages stays still. */
export function DrawnCheck({
  active = true,
  drawOnMount = false,
  className,
}: {
  active?: boolean
  drawOnMount?: boolean
  className?: string
}) {
  const reduceMotion = useReducedMotion()
  return (
    <svg
      className={className}
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <motion.path
        d="m5 12 4 4L19 6"
        initial={drawOnMount && !reduceMotion ? { pathLength: 0 } : false}
        animate={{ pathLength: active ? 1 : 0 }}
        transition={{
          duration: reduceMotion ? 0 : 0.24,
          ease: [0.2, 0.8, 0.3, 1],
        }}
      />
    </svg>
  )
}

export function SendGlyph() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <motion.path
        d="M12 19V5m-6 6 6-6 6 6"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        variants={{ rest: { y: 0 }, hover: { y: -1.5 }, pressed: { y: 0.5 } }}
        transition={{ duration: 0.16, ease: [0.2, 0.8, 0.3, 1] }}
      />
    </svg>
  )
}
