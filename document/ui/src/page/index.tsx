/**
 * The document page: drop any office document in and see what an agent sees.
 *
 * The layout follows the decision a caller makes. The verdict comes first —
 * is this a document at all, and how sure is that answer — because it decides
 * whether anything else runs. The markdown comes next, since it is what the
 * model is handed. The images come last: they are the part markdown throws
 * away, and the reason a deck of diagrams reads as empty without them.
 */

import {
  Badge,
  Button,
  CodeEditor,
  EmptyState,
  MarkdownPreview,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  type Host,
} from '@iii-dev/console-ui'
import { useCallback, useRef, useState } from 'react'

import {
  detectedFromMeaning,
  formatLabel,
  read,
  type Reading,
} from '../lib/api'

type State =
  | { status: 'idle' }
  | { status: 'reading'; name: string }
  | { status: 'done'; name: string; result: Reading }
  | { status: 'failed'; name: string; error: string }

export function DocumentPage({ host }: { host: Host }) {
  const [state, setState] = useState<State>({ status: 'idle' })
  const [dragging, setDragging] = useState(false)
  const input = useRef<HTMLInputElement>(null)

  const run = useCallback(
    async (file: File) => {
      setState({ status: 'reading', name: file.name })
      try {
        const result = await read(host.iii, file)
        setState({ status: 'done', name: file.name, result })
      } catch (error) {
        setState({
          status: 'failed',
          name: file.name,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    },
    [host.iii],
  )

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault()
      setDragging(false)
      const file = event.dataTransfer.files?.[0]
      if (file) void run(file)
    },
    [run],
  )

  // A failed read still counts as loaded: the error panel is the result, and
  // reverting to a full-height drop zone above it would bury the explanation.
  const hasResult = state.status === 'done' || state.status === 'failed'
  const loadedName = hasResult ? state.name : null

  return (
    <div className="document-ui">
      <header className="document-ui__head">
        <h1 className="document-ui__title">documents</h1>
        <p className="document-ui__lede">
          Convert a Word, PowerPoint, Excel, OpenDocument, RTF, EPUB or CSV file
          to the markdown an agent would receive, and see the images inside it.
          Conversion runs in the worker on this machine.
        </p>
      </header>

      {/* The drop zone owns the page until a document is loaded, then shrinks
          to a bar: once there are results, they are what the page is for. */}
      <div
        className={[
          'document-ui__drop',
          dragging ? 'document-ui__drop--over' : '',
          hasResult ? 'document-ui__drop--compact' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      >
        <p className="document-ui__drop-label">
          {state.status === 'reading' ? (
            `reading ${state.name}`
          ) : hasResult ? (
            <>
              <span className="document-ui__drop-file">{loadedName}</span>
              <span className="document-ui__drop-hint">
                {dragging ? 'drop to replace' : 'or drop another here'}
              </span>
            </>
          ) : (
            'Drop a document here'
          )}
        </p>
        <Button
          variant={hasResult ? 'ghost' : 'pill'}
          onClick={() => input.current?.click()}
          disabled={state.status === 'reading'}
        >
          {hasResult ? 'Read another' : 'Choose a file'}
        </Button>
        <input
          ref={input}
          type="file"
          className="document-ui__file"
          onChange={(e) => {
            const file = e.target.files?.[0]
            if (file) void run(file)
            e.target.value = ''
          }}
        />
      </div>

      {state.status === 'failed' && (
        <StatusPanel
          variant="alert"
          headline={`Could not read ${state.name}`}
          detail={state.error}
        />
      )}

      {state.status === 'idle' && (
        <EmptyState
          title="Nothing read yet"
          description="Drop a document above to convert it and see the markdown an agent would receive."
        />
      )}

      {state.status === 'done' && (
        <Result name={state.name} result={state.result} />
      )}
    </div>
  )
}

function Result({ name, result }: { name: string; result: Reading }) {
  const { detect, markdown, assets } = result

  if (!detect.format || !markdown) {
    return (
      <StatusPanel
        variant="info"
        headline="Not a document this worker reads"
        detail={`Nothing in ${name} matched a known format, and its name did not name one either. Images, archives and plain text are not converted here.`}
      />
    )
  }

  const chars = markdown.body.total_chars
  const totalMs = detect.elapsed_ms + markdown.elapsed_ms + (assets?.elapsed_ms ?? 0)
  const images = assets?.assets ?? []

  return (
    <section className="document-ui__result">
      <div className="document-ui__verdict">
        <Badge variant="accent">{formatLabel(detect.format)}</Badge>
        <span className="document-ui__verdict-meaning">
          {detect.detected_from ? detectedFromMeaning(detect.detected_from) : ''}
        </span>
      </div>

      <dl className="document-ui__stats">
        <Stat label="family" value={markdown.family.replace('_', ' ')} />
        <Stat label="size" value={formatBytes(detect.size_bytes)} />
        <Stat label="characters" value={chars.toLocaleString('en-US')} />
        <Stat label="images" value={String(markdown.asset_count)} />
        <Stat label="detect" value={`${detect.elapsed_ms} ms`} />
        <Stat label="convert" value={`${markdown.elapsed_ms} ms`} />
      </dl>

      <p className="document-ui__timing">
        {chars.toLocaleString('en-US')} characters in {totalMs} ms. Converted on
        this machine, with nothing uploaded.
      </p>

      {markdown.body.text.trim().length === 0 && (
        <StatusPanel
          variant="warn"
          headline="Converted to nothing"
          detail={
            markdown.asset_count > 0
              ? 'This document holds no text an agent can read — its content is the images below. Hand those to a model that can see them.'
              : 'This document holds no text and no images. It may be empty, or its content may be in a part this converter does not read.'
          }
        />
      )}

      <Tabs defaultValue="document" className="document-ui__tabs">
        <TabsList>
          <TabsTrigger value="document">document</TabsTrigger>
          <TabsTrigger value="source">markdown source</TabsTrigger>
          <TabsTrigger value="images">images ({images.length})</TabsTrigger>
        </TabsList>

        <TabsContent value="document" className="document-ui__pane">
          <MarkdownPreview markdown={markdown.body.text} />
        </TabsContent>

        <TabsContent value="source" className="document-ui__pane">
          {/* The console's one code editor, read-only. Never bundle another. */}
          <CodeEditor
            value={markdown.body.text}
            onChange={() => {}}
            language="markdown"
            readOnly
            aria-label="Converted markdown source"
            className="document-ui__editor"
          />
        </TabsContent>

        <TabsContent value="images" className="document-ui__pane">
          {images.length === 0 ? (
            <p className="document-ui__timing">
              No images in this document.
            </p>
          ) : (
            <>
              {assets?.truncated && (
                <p className="document-ui__timing">
                  Showing {images.length} of {assets.total_count}. The rest were
                  left out by the response ceiling.
                </p>
              )}
              <div className="document-ui__assets">
                {images.map((asset) => (
                  <figure key={asset.index} className="document-ui__asset">
                    {asset.bytes_base64 ? (
                      <img
                        className="document-ui__asset-img"
                        src={`data:${asset.media_type};base64,${asset.bytes_base64}`}
                        alt={asset.origin_part}
                      />
                    ) : (
                      <div className="document-ui__asset-missing">
                        {asset.omitted === 'too_large'
                          ? 'over the per-asset size ceiling'
                          : 'bytes not requested'}
                      </div>
                    )}
                    <figcaption className="document-ui__asset-meta">
                      {asset.media_type} · {formatBytes(asset.size_bytes)}
                    </figcaption>
                  </figure>
                ))}
              </div>
            </>
          )}
        </TabsContent>
      </Tabs>
    </section>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="document-ui__stat">
      <dt className="document-ui__stat-label">{label}</dt>
      <dd className="document-ui__stat-value">{value}</dd>
    </div>
  )
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} kb`
  return `${(bytes / (1024 * 1024)).toFixed(1)} mb`
}
