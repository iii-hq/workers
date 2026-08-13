/**
 * The mermaid half of the page: the console's shared Monaco CodeEditor on
 * one side, a live pan/zoom preview on the other (the split stacks under
 * narrow containers — see styles.css).
 *
 * Preview pipeline: debounce 300ms → loadMermaid (vendor bundle, lazy) →
 * mermaid.parse first — a parse error keeps the last good SVG on screen and
 * fills the diagnostics strip (the preview never blanks mid-typing) — then
 * mermaid.render to an SVG string, innerHTML'd into the transform canvas.
 * Sequencing and last-good-SVG rules live in preview.ts, pure and tested.
 *
 * Mermaid is initialized once per theme (host.useTheme()); a theme flip
 * re-initializes and re-renders the open diagram.
 *
 * Pan/zoom follows the house pattern (database ErdPanel): the transform is
 * written straight onto a ref'd container during pointermove and wheel —
 * state only commits when the gesture ends, so nothing re-renders per frame.
 *
 * Drafts live in a page-owned cache keyed by canvas id, so switching
 * canvases keeps unsaved edits (shell EditorPane pattern) — the pane is
 * remounted per record (key=id) and rehydrates from the cache.
 */

import { Button, CodeEditor, type Host } from '@iii-dev/console-ui'
import type {
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  EXPORT_BG,
  initMermaidOnce,
  invalidateMermaidInit,
  loadMermaid,
  mermaidInitConfig,
} from '../lib/loaders'
import { animateSvgDrawIn } from '../lib/draw'
import type { CanvasRecord } from '../lib/types'
import { updateCanvas } from './data'
import {
  downloadSvg,
  downloadSvgAsPng,
  svgDimensions,
  svgWithBackground,
} from './export'
import { ExportMenu, type ExportOptions } from './ExportMenu'
import { errorMessage, exportFilename } from './helpers'
import { AlertCircle, Maximize } from './icons'
import {
  INITIAL_PREVIEW,
  beginRender,
  renderFailed,
  renderSucceeded,
} from './preview'

const RENDER_DEBOUNCE_MS = 300

const SPLIT_PCT_KEY = 'canvas-ui:split-pct'

function clampSplit(pct: number): number {
  return Math.min(72, Math.max(24, Math.round(pct * 10) / 10))
}
const MIN_ZOOM = 0.1
const MAX_ZOOM = 4
const PAD = 48

export interface DraftEntry {
  /** What the worker last returned (or accepted) for this canvas. */
  savedSource: string
  /** The live buffer, possibly unsaved. */
  draft: string
}

/** Page-owned; survives canvas switches for the life of the page. */
export type DraftCache = Map<string, DraftEntry>

interface View {
  x: number
  y: number
  z: number
}


interface MermaidPaneProps {
  host: Host
  record: CanvasRecord
  cache: DraftCache
  /** Fired with the updated record after a successful save. */
  onSaved: (record: CanvasRecord) => void
  /** Dirty-flag transitions — the page shows dots in the sidebar. */
  onDirtyChange: (id: string, dirty: boolean) => void
}

export function MermaidPane({
  host,
  record,
  cache,
  onSaved,
  onDirtyChange,
}: MermaidPaneProps) {
  const theme = host.useTheme()

  // ── draft + save (cache-hydrated; the page seeds the entry on select) ──
  const [draft, setDraftState] = useState(
    () => cache.get(record.id)?.draft ?? record.source,
  )
  const [savedSource, setSavedSource] = useState(
    () => cache.get(record.id)?.savedSource ?? record.source,
  )
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const dirty = draft !== savedSource
  const canSave = dirty && !saving

  const setDraft = useCallback(
    (next: string) => {
      setDraftState(next)
      const entry = cache.get(record.id)
      if (entry) {
        const wasDirty = entry.draft !== entry.savedSource
        entry.draft = next
        const isDirty = next !== entry.savedSource
        if (wasDirty !== isDirty) onDirtyChange(record.id, isDirty)
      } else {
        cache.set(record.id, { savedSource: record.source, draft: next })
        if (next !== record.source) onDirtyChange(record.id, true)
      }
    },
    [cache, record.id, record.source, onDirtyChange],
  )

  const save = useCallback(() => {
    const entry = cache.get(record.id)
    if (!entry || saving || entry.draft === entry.savedSource) return
    const body = entry.draft
    setSaving(true)
    setSaveError(null)
    updateCanvas(host, record.id, body)
      .then((updated) => {
        entry.savedSource = body
        setSavedSource(body)
        onDirtyChange(record.id, entry.draft !== body)
        onSaved(updated)
      })
      .catch((err: unknown) => {
        setSaveError(errorMessage(err))
      })
      .finally(() => setSaving(false))
  }, [host, cache, record.id, saving, onSaved, onDirtyChange])

  // ── editor|preview split ratio (drag handle between the halves) ──
  const [splitPct, setSplitPct] = useState(() => {
    const stored = Number(window.localStorage.getItem(SPLIT_PCT_KEY))
    return Number.isFinite(stored) && stored > 0 ? clampSplit(stored) : 44
  })
  const splitRef = useRef<HTMLDivElement | null>(null)
  const splitDragRef = useRef(false)
  const onSplitPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      splitDragRef.current = true
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        // capture is best-effort
      }
    },
    [],
  )
  const onSplitPointerMove = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!splitDragRef.current) return
      const rect = splitRef.current?.getBoundingClientRect()
      if (!rect || rect.width === 0) return
      setSplitPct(clampSplit(((e.clientX - rect.left) / rect.width) * 100))
    },
    [],
  )
  const splitPctRef = useRef(splitPct)
  splitPctRef.current = splitPct
  const onSplitPointerUp = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (splitDragRef.current) {
        window.localStorage.setItem(SPLIT_PCT_KEY, String(splitPctRef.current))
      }
      splitDragRef.current = false
      try {
        e.currentTarget.releasePointerCapture(e.pointerId)
      } catch {
        // never captured
      }
    },
    [],
  )

  // One handler at the pane root: keydown bubbles out of Monaco, so ⌘S /
  // ctrl-S saves from either half of the split.
  const onKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        save()
      }
    },
    [save],
  )

  // ── preview pipeline ──
  const [preview, setPreview] = useState(INITIAL_PREVIEW)
  const seqRef = useRef(0)

  const renderNow = useCallback(
    async (source: string) => {
      const seq = ++seqRef.current
      setPreview((s) => beginRender(s, seq))
      try {
        const { mermaid } = await loadMermaid(host)
        initMermaidOnce(mermaid, theme)
        await mermaid.parse(source)
        if (seqRef.current !== seq) return
        const { svg } = await mermaid.render(`cv-mmd-${record.id}-${seq}`, source)
        setPreview((s) => renderSucceeded(s, seq, svg))
      } catch (err) {
        setPreview((s) => renderFailed(s, seq, errorMessage(err)))
      }
    },
    [host, theme, record.id],
  )

  // Debounced re-render on edits; theme changes recreate renderNow, which
  // re-runs the effect and re-renders the open diagram.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void renderNow(draft)
    }, RENDER_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [draft, renderNow])

  // ── svg mount + measurement ──
  const svgHostRef = useRef<HTMLDivElement | null>(null)
  const [contentSize, setContentSize] = useState<{
    w: number
    h: number
  } | null>(null)

  useEffect(() => {
    const el = svgHostRef.current
    if (!el) return
    if (preview.svg === null) {
      el.innerHTML = ''
      return
    }
    el.innerHTML = preview.svg
    animateSvgDrawIn(el)
    const svg = el.querySelector('svg')
    if (!svg) return
    const box = svg.viewBox.baseVal
    const w = box && box.width > 0 ? box.width : svg.getBoundingClientRect().width
    const h =
      box && box.height > 0 ? box.height : svg.getBoundingClientRect().height
    if (w > 0 && h > 0) {
      setContentSize((prev) =>
        prev && prev.w === w && prev.h === h ? prev : { w, h },
      )
    }
  }, [preview.svg])

  // ── pan/zoom (transform on a ref during the gesture, house pattern) ──
  const [view, setView] = useState<View>({ x: 0, y: 0, z: 1 })
  const viewRef = useRef<View>(view)
  const canvasRef = useRef<HTMLDivElement | null>(null)
  const frameRef = useRef<HTMLDivElement | null>(null)
  const dragRef = useRef<{ px: number; py: number } | null>(null)
  const rafRef = useRef<number | null>(null)

  const apply = useCallback(() => {
    rafRef.current = null
    const el = canvasRef.current
    if (!el) return
    const v = viewRef.current
    el.style.transform = `translate(${v.x}px, ${v.y}px) scale(${v.z})`
  }, [])

  const schedule = useCallback(() => {
    if (rafRef.current == null) rafRef.current = requestAnimationFrame(apply)
  }, [apply])

  const setViewNow = useCallback(
    (next: View) => {
      viewRef.current = next
      setView(next)
      schedule()
    },
    [schedule],
  )

  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current)
    },
    [],
  )

  /** Scale the diagram to fit the frame, centered. */
  const fitToFrame = useCallback(() => {
    const frame = frameRef.current
    if (!frame || !contentSize) return
    const z = Math.min(
      1,
      Math.max(
        MIN_ZOOM,
        Math.min(
          (frame.clientWidth - PAD) / contentSize.w,
          (frame.clientHeight - PAD) / contentSize.h,
        ),
      ),
    )
    setViewNow({
      x: Math.max(PAD / 2, (frame.clientWidth - contentSize.w * z) / 2),
      y: Math.max(PAD / 2, (frame.clientHeight - contentSize.h * z) / 2),
      z,
    })
  }, [contentSize, setViewNow])

  /** 100%: true scale, content centered on the frame. */
  const actualSize = useCallback(() => {
    const frame = frameRef.current
    if (!frame || !contentSize) return
    setViewNow({
      x: (frame.clientWidth - contentSize.w) / 2,
      y: (frame.clientHeight - contentSize.h) / 2,
      z: 1,
    })
  }, [contentSize, setViewNow])

  // Auto-fit once per canvas, when the first successful render is measured.
  const fittedRef = useRef(false)
  useEffect(() => {
    if (!fittedRef.current && contentSize) {
      fittedRef.current = true
      fitToFrame()
    }
  }, [contentSize, fitToFrame])

  /**
   * Zoom on wheel, around the cursor. Bound natively with `{passive: false}`
   * — React attaches wheel handlers passively, where preventDefault is a
   * no-op, and the page would scroll under the diagram instead.
   */
  useEffect(() => {
    const frame = frameRef.current
    if (!frame) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const rect = frame.getBoundingClientRect()
      const v = viewRef.current
      const next = Math.min(
        MAX_ZOOM,
        Math.max(MIN_ZOOM, v.z * (e.deltaY < 0 ? 1.12 : 1 / 1.12)),
      )
      const cx = e.clientX - rect.left
      const cy = e.clientY - rect.top
      const k = next / v.z
      setViewNow({ x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k, z: next })
    }
    frame.addEventListener('wheel', onWheel, { passive: false })
    return () => frame.removeEventListener('wheel', onWheel)
  }, [setViewNow])

  const onPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    // The diagnostics strip scrolls its own overflow — don't pan from it.
    if ((e.target as HTMLElement).closest('.cv-diag')) return
    dragRef.current = { px: e.clientX, py: e.clientY }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }

  const onPointerMove = (e: ReactPointerEvent) => {
    const d = dragRef.current
    if (!d) return
    const v = viewRef.current
    viewRef.current = { ...v, x: v.x + (e.clientX - d.px), y: v.y + (e.clientY - d.py) }
    dragRef.current = { px: e.clientX, py: e.clientY }
    schedule()
  }

  const onPointerUp = (e: ReactPointerEvent) => {
    if (!dragRef.current) return
    dragRef.current = null
    ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
    // Commit once, at the end, rather than on every frame.
    setView(viewRef.current)
  }

  // ── export ──
  // Re-renders the CURRENT DRAFT under the requested export theme (the
  // preview's global init is restored after), so light/dark come out
  // right regardless of the console theme.
  const [exportError, setExportError] = useState<string | null>(null)
  const canExport = preview.svg !== null && contentSize !== null

  const exportDiagram = useCallback(
    async ({ format, dark, background }: ExportOptions) => {
      setExportError(null)
      try {
        const { mermaid } = await loadMermaid(host)
        const exportTheme = dark ? 'dark' : 'light'
        let svg: string
        try {
          mermaid.initialize(mermaidInitConfig(exportTheme))
          invalidateMermaidInit()
          ;({ svg } = await mermaid.render(
            `cv-export-${record.id}-${++seqRef.current}`,
            cache.get(record.id)?.draft ?? record.source,
          ))
        } finally {
          initMermaidOnce(mermaid, theme)
        }
        const dims = svgDimensions(svg) ?? contentSize
        if (!dims) throw new Error('the diagram has no measurable size')
        const bg = background ? EXPORT_BG[exportTheme] : undefined
        if (format === 'svg') {
          downloadSvg(
            bg ? svgWithBackground(svg, bg) : svg,
            exportFilename(record.name, 'svg'),
          )
        } else {
          await downloadSvgAsPng(
            svg,
            dims.w,
            dims.h,
            exportFilename(record.name, 'png'),
            bg,
          )
        }
      } catch (err) {
        setExportError(errorMessage(err))
      }
    },
    [host, theme, cache, record.id, record.name, record.source, contentSize],
  )

  return (
    <div className="cv-pane" onKeyDown={onKeyDown}>
      <div className="cv-toolbar">
        <span className="cv-title" title={`${record.name} · ${record.id}`}>
          {record.name}
        </span>
        {dirty ? <span className="cv-dirty" title="unsaved changes" /> : null}
        <span className="cv-spacer" />
        {saveError ? (
          <span className="cv-note-inline warn" title={saveError}>
            save failed: {saveError}
          </span>
        ) : null}
        {exportError ? (
          <span className="cv-note-inline warn" title={exportError}>
            export failed: {exportError}
          </span>
        ) : null}
        <span className="cv-zoom">{Math.round(view.z * 100)}%</span>
        <Button
          variant="ghost"
          size="sm"
          onClick={fitToFrame}
          disabled={!contentSize}
          title="fit the diagram in the frame"
        >
          <Maximize size={13} aria-hidden />
          fit
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={actualSize}
          disabled={!contentSize}
          title="zoom to actual size"
        >
          100%
        </Button>
        <ExportMenu
          theme={theme}
          disabled={!canExport}
          onExport={(opts) => {
            void exportDiagram(opts)
          }}
        />
        <Button
          variant="primary"
          size="sm"
          onClick={save}
          disabled={!canSave}
          title="save (⌘S)"
        >
          {saving ? 'saving…' : 'save'}
        </Button>
      </div>

      <div
        className="cv-split"
        ref={splitRef}
        style={{ '--cv-split': `${splitPct}%` } as React.CSSProperties}
      >
        <div className="cv-editor">
          <CodeEditor
            value={draft}
            onChange={setDraft}
            language="plaintext"
            aria-label={`edit ${record.name}`}
            className="cv-code"
          />
        </div>

        <div
          className="cv-split-handle"
          onPointerDown={onSplitPointerDown}
          onPointerMove={onSplitPointerMove}
          onPointerUp={onSplitPointerUp}
          role="separator"
          aria-orientation="vertical"
          aria-label="resize editor and preview"
          title="drag to resize"
        />

        <div
          className="cv-preview"
          ref={frameRef}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          role="application"
          aria-label="diagram preview — drag to pan, scroll to zoom"
        >
          <div
            className="cv-preview-canvas"
            ref={canvasRef}
            style={{
              width: contentSize?.w,
              height: contentSize?.h,
              transform: `translate(${view.x}px, ${view.y}px) scale(${view.z})`,
            }}
          >
            <div className="cv-svg-host" ref={svgHostRef} />
          </div>

          {preview.svg === null ? (
            <div className="cv-preview-empty">
              {preview.rendering || preview.error === null
                ? 'rendering…'
                : 'nothing rendered yet — fix the source to see a preview'}
            </div>
          ) : null}

          {preview.error !== null ? (
            <div className="cv-diag" role="status">
              <AlertCircle size={13} aria-hidden className="cv-diag-icon" />
              <div className="cv-diag-body">
                {preview.errorLine !== null ? (
                  <span className="cv-diag-line">line {preview.errorLine}</span>
                ) : null}
                <pre className="cv-diag-msg">{preview.error}</pre>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}
