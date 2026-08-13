/**
 * The freeform (excalidraw) surface: mounting, scene wiring, dirty tracking,
 * and PNG/SVG export.
 *
 * `FreeformPane` is the contract the page codes against:
 *
 *   <FreeformPane host={host} record={record} onSave={(source) => …} />
 *
 * - `record.source` (excalidraw scene JSON) becomes the initial scene;
 *   empty/invalid source opens a blank whiteboard (see ./scene.ts).
 * - Edits auto-save: every excalidraw change arms a debounce, and when it
 *   settles the scene is serialized (`serializeAsJSON`) and pushed through
 *   `onSave(source)` — only when it actually differs from the last save.
 * - `handleRef` receives the imperative surface (`flush`, `exportPng`,
 *   `exportSvg`, `getSceneJson`, `isDirty`) once excalidraw is live, and
 *   `null` on unmount. There is NO auto-flush on unmount — a page switching
 *   records should `flush()` first (deleting a record must not resurrect it
 *   through a stray save).
 * - Mermaid→freeform conversion is NOT here: the page calls
 *   `convertMermaidToScene` off `loadFreeform(host)` (see ./convert.ts) and
 *   creates a new freeform record from the result.
 *
 * Everything excalidraw arrives through `loadFreeform(host)`
 * (../lib/loaders.ts); importing the vendor packages directly from here would
 * bundle them into page.js and blow the console's 8 MiB asset cap. Only
 * TYPE-only imports of the vendor packages are allowed — they erase at
 * compile time.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { Button, type Host } from '@iii-dev/console-ui'
import type {
  ExcalidrawImperativeAPI,
  ExcalidrawInitialDataState,
} from '@excalidraw/excalidraw/types'

import { EXPORT_BG, loadFreeform, type FreeformBundle } from '../lib/loaders'
import type { CanvasRecord } from '../lib/types'
import { downloadBlob, downloadSvg } from '../page/export'
import { ExportMenu, type ExportOptions } from '../page/ExportMenu'
import { exportFilename, parseSceneSource } from './scene'

/** How long the whiteboard must stay quiet before a change is saved. */
const SAVE_DEBOUNCE_MS = 700

/** The imperative surface `handleRef` receives (see `FreeformPaneProps`). */
export interface FreeformPaneHandle {
  /**
   * Serialize now and push through `onSave` if the scene changed since the
   * last save (any pending debounce is cancelled). Returns true when a save
   * was emitted. Call before switching away from a dirty record.
   */
  flush(): boolean
  /** The current scene as canonical excalidraw scene JSON; null before mount. */
  getSceneJson(): string | null
  /** True while edits exist that have not been pushed through `onSave` yet. */
  isDirty(): boolean
  /** PNG export with the scene embedded (`appState.exportEmbedScene`), so the
      file re-imports into any excalidraw as an editable scene. */
  exportPng(opts?: FreeformExportOptions): Promise<{ blob: Blob; filename: string }>
  /** Plain standalone SVG markup (no embedded scene). */
  exportSvg(opts?: FreeformExportOptions): Promise<{ svg: string; filename: string }>
}

/** Export look, independent of the console theme. */
export interface FreeformExportOptions {
  dark: boolean
  /** Paint the theme background; off = transparent. */
  background: boolean
}

/** A serialized element excalidraw itself produced (vs an agent skeleton). */
function isFullElement(el: unknown): boolean {
  return (
    typeof el === 'object' &&
    el !== null &&
    ('versionNonce' in el || 'seed' in el)
  )
}

/**
 * Make any stored element list renderable: full elements pass through,
 * agent skeletons run through excalidraw's converter. Converted one batch
 * per skeleton RUN (arrows may bind to ids created in the same add), and
 * a batch the converter rejects is dropped rather than sinking the whole
 * scene — rendering most of a board beats rendering none of it.
 */
function normalizeElements(
  bundle: FreeformBundle,
  elements: unknown[],
): unknown[] {
  type Convertible = Parameters<typeof bundle.convertToExcalidrawElements>[0]
  const out: unknown[] = []
  let batch: unknown[] = []
  const flushBatch = () => {
    if (batch.length === 0) return
    try {
      out.push(
        ...bundle.convertToExcalidrawElements(batch as Convertible, {
          regenerateIds: false,
        }),
      )
    } catch {
      // Skip only the unconvertible batch.
    }
    batch = []
  }
  for (const el of elements) {
    if (isFullElement(el)) {
      flushBatch()
      out.push(el)
    } else {
      batch.push(el)
    }
  }
  flushBatch()
  return out
}

export interface FreeformPaneProps {
  host: Host
  /** The freeform canvas being edited; `source` is excalidraw scene JSON. */
  record: CanvasRecord
  /** Receives the serialized scene after each settled, genuinely-new change. */
  onSave(source: string): void
  /** Imperative surface, delivered once live and revoked (null) on unmount. */
  handleRef?: (handle: FreeformPaneHandle | null) => void
  /** Observe the dirty flag, e.g. for an unsaved-changes dot. */
  onDirtyChange?: (dirty: boolean) => void
}

/**
 * Lazy-loads the excalidraw vendor bundle, then mounts the editor. Loading
 * and failure states render lightweight placeholders; a failed load can be
 * retried (loadFreeform drops its memo on rejection).
 */
export function FreeformPane(props: FreeformPaneProps) {
  const { host } = props
  const [bundle, setBundle] = useState<FreeformBundle | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const theme = host.useTheme()

  useEffect(() => {
    let alive = true
    setLoadError(null)
    loadFreeform(host).then(
      (loaded) => {
        if (alive) setBundle(loaded)
      },
      (err: unknown) => {
        if (alive) setLoadError(err instanceof Error ? err.message : String(err))
      },
    )
    return () => {
      alive = false
    }
  }, [host, attempt])

  if (loadError !== null) {
    return (
      <div className="canvas-freeform canvas-freeform--status">
        <span className="canvas-freeform__status">
          whiteboard failed to load: {loadError}
        </span>
        <Button variant="ghost" size="sm" onClick={() => setAttempt((n) => n + 1)}>
          retry
        </Button>
      </div>
    )
  }
  if (bundle === null) {
    return (
      <div className="canvas-freeform canvas-freeform--status">
        <span className="canvas-freeform__status">loading whiteboard…</span>
      </div>
    )
  }
  // key: a different record must remount excalidraw with its own initialData —
  // excalidraw treats initialData as mount-time-only.
  return (
    <FreeformEditor
      key={props.record.id}
      bundle={bundle}
      theme={theme}
      {...props}
    />
  )
}

interface FreeformEditorProps extends FreeformPaneProps {
  bundle: FreeformBundle
  theme: 'light' | 'dark'
}

function FreeformEditor({
  bundle,
  theme,
  record,
  onSave,
  handleRef,
  onDirtyChange,
}: FreeformEditorProps) {
  // Parsed once per mount (the record.id key above remounts on record change);
  // later parent refreshes of the same record must not reset the live scene.
  // Elements run through the skeleton converter FIRST: canvas::element::add
  // stores skeleton shapes verbatim, and excalidraw's restore hangs on a
  // bare {type, label} skeleton handed to it as initialData.
  const [initialData] = useState(() => {
    const parsed = parseSceneSource(record.source)
    return {
      ...parsed,
      elements: normalizeElements(bundle, parsed.elements),
    }
  })

  const apiRef = useRef<ExcalidrawImperativeAPI | null>(null)
  /** The serialized scene as of the last onSave (or the mount baseline). */
  const lastSavedRef = useRef<string | null>(null)
  const dirtyRef = useRef(false)
  const timerRef = useRef<number | null>(null)

  // Latest-callback refs keep the excalidraw props/handle stable across parent
  // re-renders without stale closures.
  const onSaveRef = useRef(onSave)
  onSaveRef.current = onSave
  const onDirtyChangeRef = useRef(onDirtyChange)
  onDirtyChangeRef.current = onDirtyChange

  const setDirty = useCallback((next: boolean) => {
    if (dirtyRef.current === next) return
    dirtyRef.current = next
    onDirtyChangeRef.current?.(next)
  }, [])

  const serializeScene = useCallback((): string | null => {
    const api = apiRef.current
    if (api === null) return null
    return bundle.serializeAsJSON(
      api.getSceneElements(),
      api.getAppState(),
      api.getFiles(),
      'local',
    )
  }, [bundle])

  const acceptApi = useCallback(
    (api: ExcalidrawImperativeAPI) => {
      apiRef.current = api
      // A blank scene never fires the restore onChange, so the baseline (what
      // "unchanged" serializes to) is captured here. Non-blank scenes baseline
      // on their first onChange instead — at api-callback time excalidraw has
      // not committed initialData yet.
      if (initialData.elements.length === 0 && lastSavedRef.current === null) {
        lastSavedRef.current = serializeScene()
      }
    },
    [initialData, serializeScene],
  )

  // ── live-apply: agent element calls stream onto the OPEN whiteboard ──
  // The parent refreshes `record` when a state event lands. A source we
  // did not draw is applied in place through updateScene (skeletons from
  // canvas::element::add run through excalidraw's converter), with
  // CaptureUpdateAction.NEVER so remote strokes never enter the user's
  // undo stack. Local dirty edits win: while the user is mid-drawing,
  // remote applies wait for the next clean refresh.
  const lastAppliedRef = useRef(record.source)
  /** A remote source that arrived while the user was mid-edit; applied on
      the next clean transition instead of being dropped. */
  const pendingRemoteRef = useRef<string | null>(null)

  const applyRemote = useCallback(
    (source: string) => {
      const api = apiRef.current
      if (api === null) return
      lastAppliedRef.current = source
      try {
        const scene = parseSceneSource(source)
        const elements = normalizeElements(bundle, scene.elements)
        api.updateScene({
          elements: elements as Parameters<
            typeof api.updateScene
          >[0] extends { elements?: infer E }
            ? E
            : never,
          captureUpdate: bundle.CaptureUpdateAction.NEVER,
        })
        // The applied scene is now the save baseline — without this, the
        // next local edit would diff against the pre-stream scene and
        // re-save what the agent just drew.
        lastSavedRef.current = serializeScene()
      } catch {
        // A malformed streamed scene falls back to the next full remount.
      }
    },
    [bundle, serializeScene],
  )

  useEffect(() => {
    if (record.source === lastAppliedRef.current) return
    if (apiRef.current === null) return
    if (dirtyRef.current) {
      pendingRemoteRef.current = record.source
      return
    }
    pendingRemoteRef.current = null
    applyRemote(record.source)
  }, [record.source, applyRemote])

  // HARD SAFETY RULE: nothing persists until the person actually touched
  // the board this mount. Excalidraw's restore fires onChange on its own
  // (and a scene it failed to restore serializes EMPTY) — without this
  // gate, merely opening a canvas the renderer choked on auto-saved the
  // choke back and destroyed the stored elements.
  const userInteractedRef = useRef(false)

  const saveIfChanged = useCallback((): boolean => {
    if (!userInteractedRef.current) return false
    const json = serializeScene()
    if (json === null) return false
    if (json === lastSavedRef.current) {
      setDirty(false)
      // Clean without writing (e.g. drew and undid): a remote update that
      // was parked during the edit lands now instead of being lost.
      const parked = pendingRemoteRef.current
      if (parked !== null) {
        pendingRemoteRef.current = null
        applyRemote(parked)
      }
      return false
    }
    lastSavedRef.current = json
    setDirty(false)
    // Our save supersedes any parked remote: the parent refreshes the
    // record from the save response, which carries the newer source.
    pendingRemoteRef.current = null
    onSaveRef.current(json)
    return true
  }, [serializeScene, setDirty, applyRemote])

  const handleChange = useCallback(() => {
    if (lastSavedRef.current === null) {
      // First scene commit after mount is excalidraw restoring initialData —
      // adopt it as the save baseline instead of echoing it through onSave.
      lastSavedRef.current = serializeScene()
      return
    }
    if (!userInteractedRef.current) return
    setDirty(true)
    if (timerRef.current !== null) window.clearTimeout(timerRef.current)
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null
      saveIfChanged()
    }, SAVE_DEBOUNCE_MS)
  }, [serializeScene, setDirty, saveIfChanged])

  const flush = useCallback((): boolean => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    return saveIfChanged()
  }, [saveIfChanged])

  /** Export appState for the requested look — background on/off, forced
      light/dark independent of the console theme. */
  const exportAppState = useCallback(
    (opts: FreeformExportOptions, embedScene: boolean) => {
      const api = apiRef.current
      if (api === null) throw new Error('freeform pane is not mounted yet')
      return {
        ...api.getAppState(),
        exportEmbedScene: embedScene,
        exportBackground: opts.background,
        exportWithDarkMode: opts.dark,
        viewBackgroundColor: opts.background
          ? EXPORT_BG[opts.dark ? 'dark' : 'light']
          : 'transparent',
      }
    },
    [],
  )

  const exportPng = useCallback(
    async (opts: FreeformExportOptions = { dark: false, background: true }) => {
      const api = apiRef.current
      if (api === null) throw new Error('freeform pane is not mounted yet')
      const blob = await bundle.exportToBlob({
        elements: api.getSceneElements(),
        // exportEmbedScene stamps the scene JSON into the PNG metadata: the
        // exported image re-imports as an editable scene.
        appState: exportAppState(opts, true),
        files: api.getFiles(),
        mimeType: 'image/png',
      })
      return { blob, filename: exportFilename(record.name, 'png') }
    },
    [bundle, record.name, exportAppState],
  )

  const exportSvg = useCallback(
    async (opts: FreeformExportOptions = { dark: false, background: true }) => {
      const api = apiRef.current
      if (api === null) throw new Error('freeform pane is not mounted yet')
      const svgElement = await bundle.exportToSvg({
        elements: api.getSceneElements(),
        appState: exportAppState(opts, false),
        files: api.getFiles(),
      })
      const svg = new XMLSerializer().serializeToString(svgElement)
      return { svg, filename: exportFilename(record.name, 'svg') }
    },
    [bundle, record.name, exportAppState],
  )

  const [exportError, setExportError] = useState<string | null>(null)
  const runExport = useCallback(
    (opts: ExportOptions) => {
      setExportError(null)
      const freeformOpts = { dark: opts.dark, background: opts.background }
      const job =
        opts.format === 'png'
          ? exportPng(freeformOpts).then(({ blob, filename }) =>
              downloadBlob(blob, filename),
            )
          : exportSvg(freeformOpts).then(({ svg, filename }) =>
              downloadSvg(svg, filename),
            )
      job.catch((err: unknown) =>
        setExportError(err instanceof Error ? err.message : String(err)),
      )
    },
    [exportPng, exportSvg],
  )

  useEffect(() => {
    if (!handleRef) return
    handleRef({
      flush,
      getSceneJson: serializeScene,
      isDirty: () => dirtyRef.current,
      exportPng,
      exportSvg,
    })
    return () => handleRef(null)
  }, [handleRef, flush, serializeScene, exportPng, exportSvg])

  // Cancel any pending debounce on unmount — deliberately WITHOUT flushing
  // (see the module comment: a deleted record must not be re-saved).
  useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current)
    }
  }, [])

  const Excalidraw = bundle.Excalidraw
  // ParsedScene is structurally a subset of ExcalidrawInitialDataState;
  // excalidraw runs every initialData through its own restore(), which is
  // built to absorb untrusted shapes — the cast is the boundary, restore is
  // the validator.
  const excalidrawInitialData = useMemo(
    () => initialData as unknown as ExcalidrawInitialDataState,
    [initialData],
  )

  return (
    <div className="canvas-freeform">
      <div className="cv-toolbar">
        <span className="cv-title" title={`${record.name} · ${record.id}`}>
          {record.name}
        </span>
        <span className="cv-spacer" />
        {exportError ? (
          <span className="cv-note-inline warn" title={exportError}>
            export failed: {exportError}
          </span>
        ) : null}
        <ExportMenu theme={theme} disabled={false} onExport={runExport} />
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: capture-phase interaction latch, not a control */}
      <div
        className="canvas-freeform-board"
        onPointerDownCapture={() => {
          userInteractedRef.current = true
        }}
        onKeyDownCapture={() => {
          userInteractedRef.current = true
        }}
      >
        <Excalidraw
          excalidrawAPI={acceptApi}
          initialData={excalidrawInitialData}
          theme={theme}
          onChange={handleChange}
        />
      </div>
    </div>
  )
}
