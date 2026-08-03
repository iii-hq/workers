/**
 * The PDF page: drop a document in and see exactly what an agent sees.
 *
 * The layout follows the decision an agent makes. The verdict comes first,
 * because it determines whether anything else is worth doing. The per-page OCR
 * grid comes next, because a mixed document is the interesting case and a
 * document-level verdict hides it. The markdown comes last, since by then you
 * already know whether to trust it.
 */

import {
  Badge,
  Button,
  EmptyState,
  MarkdownPreview,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  type Host,
} from '@iii-dev/console-ui'
import { useCallback, useMemo, useRef, useState } from 'react'

import {
  documentTypeLabel,
  documentTypeMeaning,
  inspect,
  ocrReasonLabel,
  type Inspection,
} from '../lib/api'

type State =
  | { status: 'idle' }
  | { status: 'reading'; name: string }
  | { status: 'done'; name: string; result: Inspection }
  | { status: 'failed'; name: string; error: string }

export function PdfPage({ host }: { host: Host }) {
  const [state, setState] = useState<State>({ status: 'idle' })
  const [dragging, setDragging] = useState(false)
  const input = useRef<HTMLInputElement>(null)

  const run = useCallback(
    async (file: File) => {
      setState({ status: 'reading', name: file.name })
      try {
        const result = await inspect(host.iii, file)
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

  return (
    <div className="pdf-ui">
      <header className="pdf-ui__head">
        <h1 className="pdf-ui__title">pdf</h1>
        <p className="pdf-ui__lede">
          Classify a document, see which pages need OCR, and read the markdown an
          agent would get. Parsing runs in the worker on this machine.
        </p>
      </header>

      <div
        className={`pdf-ui__drop${dragging ? ' pdf-ui__drop--over' : ''}`}
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      >
        <p className="pdf-ui__drop-label">
          {state.status === 'reading'
            ? `reading ${state.name}`
            : 'Drop a PDF here'}
        </p>
        <Button
          variant="pill"
          onClick={() => input.current?.click()}
          disabled={state.status === 'reading'}
        >
          Choose a file
        </Button>
        <input
          ref={input}
          type="file"
          accept="application/pdf,.pdf"
          className="pdf-ui__file"
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
          description="Drop a PDF above to classify it and see the markdown an agent would receive."
        />
      )}

      {state.status === 'done' && <Result name={state.name} result={state.result} />}
    </div>
  )
}

function Result({ name, result }: { name: string; result: Inspection }) {
  const { classify, markdown } = result

  const ocrByPage = useMemo(() => {
    const map = new Map<number, string[]>()
    for (const entry of classify.ocr_reasons) map.set(entry.page, entry.reasons)
    return map
  }, [classify.ocr_reasons])

  const tables = new Set(markdown?.pages_with_tables ?? [])
  const columns = new Set(markdown?.pages_with_columns ?? [])
  const totalMs = classify.elapsed_ms + (markdown?.elapsed_ms ?? 0)

  return (
    <section className="pdf-ui__result">
      <div className="pdf-ui__verdict">
        <Badge variant={badgeVariant(classify.document_type)}>
          {documentTypeLabel(classify.document_type)}
        </Badge>
        <span className="pdf-ui__verdict-meaning">
          {documentTypeMeaning(classify.document_type)}
        </span>
      </div>

      <dl className="pdf-ui__stats">
        <Stat label="confidence" value={`${Math.round(classify.confidence * 100)}%`} />
        <Stat label="pages" value={String(classify.page_count)} />
        <Stat
          label="sampled"
          value={`${classify.pages_sampled} of ${classify.page_count}`}
        />
        <Stat label="need ocr" value={String(classify.pages_needing_ocr.length)} />
        <Stat label="elapsed" value={`${totalMs} ms`} />
      </dl>

      {classify.title && <p className="pdf-ui__doc-title">{classify.title}</p>}

      {markdown?.has_encoding_issues && (
        <StatusPanel
          variant="warn"
          headline="Font encodings decoded badly"
          detail="The markdown below is not trustworthy. Treat this document as one that needs OCR."
        />
      )}

      {classify.pages_needing_ocr.length > 0 && (
        <div className="pdf-ui__ocr">
          <h2 className="pdf-ui__section">Pages needing OCR</h2>
          <ul className="pdf-ui__ocr-list">
            {classify.pages_needing_ocr.map((page) => (
              <li key={page} className="pdf-ui__ocr-row">
                <span className="pdf-ui__ocr-page">page {page}</span>
                <span className="pdf-ui__ocr-why">
                  {(ocrByPage.get(page) ?? []).map(ocrReasonLabel).join('; ') ||
                    'no reason recorded'}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {markdown ? (
        <Tabs defaultValue="document" className="pdf-ui__tabs">
          <TabsList>
            <TabsTrigger value="document">document</TabsTrigger>
            <TabsTrigger value="pages">
              pages{markdown.pages ? ` (${markdown.pages.length})` : ''}
            </TabsTrigger>
            <TabsTrigger value="source">markdown source</TabsTrigger>
          </TabsList>

          <TabsContent value="document" className="pdf-ui__pane">
            <MarkdownPreview markdown={markdown.body.text} />
          </TabsContent>

          <TabsContent value="pages" className="pdf-ui__pane">
            {(markdown.pages ?? []).map((page) => (
              <article key={page.page} className="pdf-ui__page">
                <header className="pdf-ui__page-head">
                  <span className="pdf-ui__page-no">page {page.page}</span>
                  {page.needs_ocr && (
                    <Badge variant="warn">
                      {page.ocr_reason ? ocrReasonLabel(page.ocr_reason) : 'needs ocr'}
                    </Badge>
                  )}
                  {tables.has(page.page) && <Badge>table</Badge>}
                  {columns.has(page.page) && <Badge>columns</Badge>}
                </header>
                <MarkdownPreview markdown={page.markdown} />
              </article>
            ))}
          </TabsContent>

          <TabsContent value="source" className="pdf-ui__pane">
            <pre className="pdf-ui__source">{markdown.body.text}</pre>
          </TabsContent>
        </Tabs>
      ) : (
        <StatusPanel
          variant="info"
          headline="No text to extract"
          detail={`${name} holds no usable text layer, so conversion was skipped. Every page would need OCR.`}
        />
      )}
    </section>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="pdf-ui__stat">
      <dt className="pdf-ui__stat-label">{label}</dt>
      <dd className="pdf-ui__stat-value">{value}</dd>
    </div>
  )
}

function badgeVariant(type: string): 'default' | 'accent' | 'warn' | 'alert' {
  switch (type) {
    case 'text_based':
      return 'accent'
    case 'mixed':
      return 'warn'
    case 'scanned':
    case 'image_based':
      return 'alert'
    default:
      return 'default'
  }
}
