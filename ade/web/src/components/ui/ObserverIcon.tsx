import type { LucideProps } from 'lucide-react'
import { forwardRef } from 'react'

/** Filled eye — the observer mark — with the same sizing and color API as a Lucide icon. */
export const ObserverIcon = forwardRef<SVGSVGElement, LucideProps>(
  ({ color, size = 24, className, ...props }, ref) => (
    // biome-ignore lint/a11y/noSvgWithoutTitle: decorative by default; aria-label is forwarded for the one labeled usage.
    <svg
      ref={ref}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fillRule="evenodd"
      clipRule="evenodd"
      fill="currentColor"
      color={color}
      className={className}
      data-icon="observer"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
      aria-hidden={props['aria-hidden'] ?? !props['aria-label']}
    >
      <path d="M12 4.5C6.6 4.5 2.73 8.3 1.25 12c1.48 3.7 5.35 7.5 10.75 7.5S21.27 15.7 22.75 12C21.27 8.3 17.4 4.5 12 4.5ZM12 16.25a4.25 4.25 0 1 0 0-8.5 4.25 4.25 0 0 0 0 8.5ZM12 14.25a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5Z" />
    </svg>
  ),
)

ObserverIcon.displayName = 'ObserverIcon'
