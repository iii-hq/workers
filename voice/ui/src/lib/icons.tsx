/**
 * Minimal 16px inline SVG glyphs shared by the chip, turn summary and page
 * (mic, speaker, copy, send, download). No icon library — everything else
 * this worker's UI needs comes from `@iii-dev/console-ui`.
 */

import type { ReactNode, SVGProps } from 'react'

function Glyph(props: SVGProps<SVGSVGElement> & { children: ReactNode }) {
  const { children, ...rest } = props
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  )
}

export function MicIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Glyph {...props}>
      <rect x="5.5" y="1.5" width="5" height="8" rx="2.5" />
      <path d="M3 7.5a5 5 0 0 0 10 0" />
      <path d="M8 12.5v2" />
      <path d="M5.5 14.5h5" />
    </Glyph>
  )
}

export function SpeakerIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Glyph {...props}>
      <path d="M2 6h2.5L8 3v10L4.5 10H2z" />
      <path d="M10.5 5.5a4 4 0 0 1 0 5" />
    </Glyph>
  )
}

export function CopyIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Glyph {...props}>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M2.5 10.5v-7a1 1 0 0 1 1-1h7" />
    </Glyph>
  )
}

export function SendIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Glyph {...props}>
      <path d="M2 8l12-5.5L9.5 14l-2-5.5L2 8z" />
    </Glyph>
  )
}

export function DownloadIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Glyph {...props}>
      <path d="M8 2v8" />
      <path d="M4.5 7 8 10.5 11.5 7" />
      <path d="M2.5 13.5h11" />
    </Glyph>
  )
}
