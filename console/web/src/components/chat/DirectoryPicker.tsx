import { AlertCircle, Check, ChevronUp, Folder, FolderOpen } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { cn } from '@/lib/utils'

/**
 * Per-session working-directory picker. Browses the operator's configured coder
 * roots (`coder::info` → `base_paths`) one level at a time (`coder::list-folder`,
 * lazy) and selects a directory the harness then scopes the chat to (`base_dir`).
 *
 * The chosen dir is fixed for the session: once the chat is locked (after the
 * first send) the trigger renders read-only. When no project roots are
 * configured the panel shows a self-documenting setup pointer rather than a
 * bare `/tmp` dead-end.
 */

interface DirectoryPickerProps {
  /** Current selection (absolute path) or null. */
  value: string | null
  onChange: (dir: string) => void
  /** Locked after the first send — render read-only, no popover. */
  locked?: boolean
  /** Editor lock (streaming / harness unavailable). */
  disabled?: boolean
  className?: string
}

interface CoderInfo {
  base_paths?: string[]
  primary_root?: string
}

interface DirEntry {
  name: string
  kind: string
  non_accessible?: boolean
}

interface ListFolderResult {
  path: string
  entries?: DirEntry[]
}

function basename(p: string): string {
  const parts = p.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

function parentOf(p: string): string {
  const trimmed = p.replace(/\/+$/, '')
  const idx = trimmed.lastIndexOf('/')
  return idx <= 0 ? '/' : trimmed.slice(0, idx)
}

export function DirectoryPicker({
  value,
  onChange,
  locked,
  disabled,
  className,
}: DirectoryPickerProps) {
  const [open, setOpen] = useState(false)
  const [roots, setRoots] = useState<string[] | null>(null)
  const [view, setView] = useState<'roots' | 'browse'>('roots')
  const [root, setRoot] = useState<string | null>(null)
  const [path, setPath] = useState<string | null>(null)
  const [dirs, setDirs] = useState<string[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  // Close on outside click.
  useEffect(() => {
    if (!open) return
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDocClick)
    return () => document.removeEventListener('mousedown', onDocClick)
  }, [open])

  // Load roots the first time the panel opens.
  useEffect(() => {
    if (!open || roots !== null) return
    let cancelled = false
    setLoading(true)
    setError(null)
    void (async () => {
      try {
        const client = await getIiiClient()
        const info = await client.trigger<CoderInfo>('coder::info', {})
        if (cancelled) return
        setRoots(info?.base_paths ?? [])
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
        setRoots([])
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [open, roots])

  const loadFolder = useCallback(async (target: string) => {
    setLoading(true)
    setError(null)
    try {
      const client = await getIiiClient()
      const res = await client.trigger<ListFolderResult>('coder::list-folder', {
        path: target,
        page_size: 200,
      })
      const names = (res?.entries ?? [])
        .filter((e) => e.kind === 'dir' && !e.non_accessible)
        .map((e) => `${target.replace(/\/+$/, '')}/${e.name}`)
        .sort((a, b) => a.localeCompare(b))
      setDirs(names)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setDirs([])
    } finally {
      setLoading(false)
    }
  }, [])

  const enterRoot = useCallback(
    (r: string) => {
      setView('browse')
      setRoot(r)
      setPath(r)
      void loadFolder(r)
    },
    [loadFolder],
  )

  const enterDir = useCallback(
    (d: string) => {
      setPath(d)
      void loadFolder(d)
    },
    [loadFolder],
  )

  const goUp = useCallback(() => {
    if (!path || !root || path === root) {
      setView('roots')
      setPath(null)
      setRoot(null)
      return
    }
    const next = parentOf(path)
    const clamped = next.length < root.length ? root : next
    setPath(clamped)
    void loadFolder(clamped)
  }, [path, root, loadFolder])

  const choose = useCallback(() => {
    if (path) {
      onChange(path)
      setOpen(false)
    }
  }, [path, onChange])

  const label = value ? basename(value) : 'choose directory'

  // Read-only chip once the session is locked.
  if (locked) {
    return (
      <span
        className={cn(
          'inline-flex items-center gap-1 px-2 py-1 text-[11px] lowercase text-ink-faint',
          className,
        )}
        title={value ?? 'no working directory'}
      >
        <Folder size={12} aria-hidden />
        <span className="max-w-[160px] truncate">{label}</span>
      </span>
    )
  }

  return (
    <div ref={containerRef} className={cn('relative inline-flex', className)}>
      <button
        type="button"
        disabled={disabled}
        aria-label="working directory"
        aria-haspopup="dialog"
        aria-expanded={open}
        title={value ?? 'choose a working directory'}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'inline-flex items-center gap-1 rounded-sm border border-rule px-2 py-1 text-[11px] lowercase transition-colors',
          value ? 'text-ink' : 'text-ink-faint',
          'hover:text-ink hover:border-rule disabled:opacity-50',
        )}
      >
        <Folder size={12} aria-hidden />
        <span className="max-w-[160px] truncate">{label}</span>
      </button>

      {open ? (
        <div
          role="dialog"
          aria-label="select working directory"
          className="absolute bottom-full left-0 z-30 mb-1 w-[340px] border border-rule bg-bg shadow-lg"
        >
          {/* header: breadcrumb / up */}
          <div className="flex items-center justify-between gap-2 border-b border-rule-2 px-2 py-1.5">
            <span className="min-w-0 flex items-center gap-1 text-[11px] text-ink-faint">
              {view === 'browse' ? (
                <button
                  type="button"
                  aria-label="up one level"
                  onClick={goUp}
                  className="text-ink-faint hover:text-ink"
                >
                  <ChevronUp size={13} aria-hidden />
                </button>
              ) : null}
              <span className="truncate font-mono">
                {view === 'browse' && path ? path : 'roots'}
              </span>
            </span>
            {view === 'browse' && path ? (
              <button
                type="button"
                onClick={choose}
                className="inline-flex items-center gap-1 whitespace-nowrap text-[11px] lowercase text-accent hover:underline"
              >
                <Check size={12} aria-hidden /> use this folder
              </button>
            ) : null}
          </div>

          {/* body */}
          <div className="max-h-[260px] overflow-y-auto py-1">
            {loading ? (
              <div className="px-3 py-2 text-[11px] lowercase text-ink-ghost">
                loading…
              </div>
            ) : error ? (
              <div className="flex items-start gap-1.5 px-3 py-2 text-[11px] text-ink-faint">
                <AlertCircle
                  size={12}
                  className="mt-0.5 shrink-0"
                  aria-hidden
                />
                <span>{error}</span>
              </div>
            ) : view === 'roots' ? (
              roots && roots.length > 0 ? (
                roots.map((r) => (
                  <button
                    key={r}
                    type="button"
                    onClick={() => enterRoot(r)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink hover:bg-panel"
                  >
                    <FolderOpen size={13} className="shrink-0" aria-hidden />
                    <span className="truncate font-mono">{r}</span>
                  </button>
                ))
              ) : (
                // DX-3: self-documenting empty-state.
                <div className="px-3 py-3 text-[11px] leading-relaxed text-ink-faint">
                  no project roots configured. set coder{' '}
                  <span className="font-mono">base_paths</span> / shell{' '}
                  <span className="font-mono">host_root</span> to your project
                  parent, then reopen.
                </div>
              )
            ) : dirs.length > 0 ? (
              dirs.map((d) => (
                <button
                  key={d}
                  type="button"
                  onClick={() => enterDir(d)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink hover:bg-panel"
                >
                  <Folder size={13} className="shrink-0" aria-hidden />
                  <span className="truncate font-mono">{basename(d)}</span>
                </button>
              ))
            ) : (
              <div className="px-3 py-2 text-[11px] lowercase text-ink-ghost">
                no sub-folders — use this folder
              </div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  )
}
