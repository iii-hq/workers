import { useCallback, useRef, useState } from 'react'

function iconProps(className?: string) {
  return {
    className,
    viewBox: '0 0 16 16',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.25,
    strokeLinecap: 'round',
    strokeLinejoin: 'round',
  } as const
}

export function StorageIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <ellipse cx="8" cy="3.5" rx="5.5" ry="2" />
      <path d="M2.5 3.5v4c0 1.1 2.46 2 5.5 2s5.5-.9 5.5-2v-4" />
      <path d="M2.5 7.5v4c0 1.1 2.46 2 5.5 2s5.5-.9 5.5-2v-4" />
    </svg>
  )
}

export function BucketIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M2.25 4.25h11.5l-1.4 8.5H3.65l-1.4-8.5Z" />
      <path d="M5 4.25V3a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v1.25" />
    </svg>
  )
}

export function FolderIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M1.75 4.25h4l1.2 1.5h7.3v6.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1v-8Z" />
      <path d="M1.75 5.75h12.5" />
    </svg>
  )
}

export function FileIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M4 1.75h5l3 3v9.5H4z" />
      <path d="M9 1.75v3h3" />
    </svg>
  )
}

export function BackIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="m9.75 3.5-4.5 4.5 4.5 4.5" />
    </svg>
  )
}

export function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M13.25 7.25A5.5 5.5 0 1 0 13 10" />
      <path d="M10.25 5.25h3v-3" />
    </svg>
  )
}

export function UploadIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M8 10.5v-8" />
      <path d="m4.75 5.75 3.25-3.25 3.25 3.25" />
      <path d="M2.5 10.25v3h11v-3" />
    </svg>
  )
}

export function DownloadIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M8 2.5v8" />
      <path d="m4.75 7.25 3.25 3.25 3.25-3.25" />
      <path d="M2.5 12.5v1h11v-1" />
    </svg>
  )
}

export function TrashIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="M3 4.25h10M6 4.25v-2h4v2M4.25 4.25l.65 9h6.2l.65-9M6.5 6.5v4.75M9.5 6.5v4.75" />
    </svg>
  )
}

export function ChevronIcon({ className }: { className?: string }) {
  return (
    <svg {...iconProps(className)} aria-hidden="true">
      <path d="m6 3.75 4.25 4.25L6 12.25" />
    </svg>
  )
}

export function useContainerNarrow(
  threshold: number,
): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const ref = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [ref, narrow]
}

export function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let size = value / 1024
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${size >= 10 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`
}

export function leafName(path: string) {
  const clean = path.endsWith('/') ? path.slice(0, -1) : path
  return clean.slice(clean.lastIndexOf('/') + 1) || clean
}

export function parentPrefix(prefix: string) {
  const clean = prefix.endsWith('/') ? prefix.slice(0, -1) : prefix
  const slash = clean.lastIndexOf('/')
  return slash < 0 ? '' : clean.slice(0, slash + 1)
}
