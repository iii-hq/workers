import { useEffect, useMemo, useState } from 'react'
import { marked } from 'marked'
import { SPEC_DOCS } from './spec-docs'

type Route = 'overview' | 'spec'
type Theme = 'dark' | 'light'
type DocId = (typeof SPEC_DOCS)[number]['id']

const SPEC_DIRECTORY = ['tech-specs', '2026-07-15-harness-evaluation']
const DOC_BY_FILENAME: Record<string, DocId> = {
  'README.md': 'overview',
  'conformance-e2e.md': 'conformance',
  'agent-quality.md': 'agent-quality',
}

const TRACKS = [
  {
    number: '01',
    label: 'conformance',
    eyebrow: 'controlled boundary',
    title: 'prove the contract',
    description:
      'A scripted router makes every generation reproducible while the real queue, transcript, context manager, and harness do the work.',
    signal: 'deterministic pass / fail',
    color: 'cyan',
  },
  {
    number: '02',
    label: 'agent quality',
    eyebrow: 'production boundary',
    title: 'measure the outcome',
    description:
      'A pinned real model runs representative workflows. Versioned validators grade durable evidence and raw metrics expose the tradeoffs.',
    signal: 'quality · reliability · cost',
    color: 'amber',
  },
] as const

const STEPS = [
  ['arm', 'Bind completion before work begins.'],
  ['send', 'Enter through harness::send.'],
  ['persist', 'Let the real queue and transcript own durability.'],
  ['observe', 'Treat events as notification, status as recovery.'],
  ['grade', 'Decide from structured evidence.'],
  ['report', 'Keep every non-pass explainable.'],
] as const

const BOUNDARIES = [
  {
    system: 'HarnessBench',
    owns: 'Same-prompt configuration comparison and console view',
    relation: 'Separate product and run record',
  },
  {
    system: 'workflow',
    owns: 'Production DAG execution, checkpoints, retries',
    relation: 'May be evaluated; not extended',
  },
  {
    system: 'harness-eval',
    owns: 'Validation cycles, held-out grading, experiment reports',
    relation: 'New dedicated evaluator',
  },
] as const

function routeFromHash(): Route {
  return window.location.hash.startsWith('#/spec') ? 'spec' : 'overview'
}

function stripFrontmatter(markdown: string) {
  return markdown.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '')
}

function normalizePath(parts: string[]) {
  return parts.reduce<string[]>((resolved, part) => {
    if (!part || part === '.') return resolved
    if (part === '..') return resolved.slice(0, -1)
    return [...resolved, part]
  }, [])
}

function rewriteSpecHref(href: string) {
  if (/^(?:[a-z]+:|\/|#)/i.test(href)) return href

  const [path, fragment] = href.split('#', 2)
  const filename = path.split('/').pop() ?? ''
  const localDoc = DOC_BY_FILENAME[filename]
  if (localDoc && !path.startsWith('../')) {
    const params = new URLSearchParams({ doc: localDoc })
    if (fragment) params.set('anchor', fragment)
    return `#/spec?${params.toString()}`
  }

  const resolved = normalizePath([...SPEC_DIRECTORY, ...path.split('/')]).join('/')
  const route = path.endsWith('/') ? 'tree' : 'blob'
  return `https://github.com/iii-hq/workers/${route}/main/${resolved}${fragment ? `#${fragment}` : ''}`
}

function renderMarkdown(content: string) {
  const renderer = new marked.Renderer()
  const renderLink = renderer.link.bind(renderer)
  const renderImage = renderer.image.bind(renderer)
  renderer.link = (token) => renderLink({ ...token, href: rewriteSpecHref(token.href) })
  renderer.image = (token) => renderImage({ ...token, href: rewriteSpecHref(token.href) })
  return marked.parse(stripFrontmatter(content), { async: false, renderer }) as string
}

function docFromHash(): DocId {
  const query = window.location.hash.split('?', 2)[1]
  const candidate = new URLSearchParams(query).get('doc')
  return SPEC_DOCS.some((doc) => doc.id === candidate) ? candidate as DocId : 'overview'
}

function useRoute() {
  const [route, setRoute] = useState<Route>(routeFromHash)
  useEffect(() => {
    const update = () => setRoute(routeFromHash())
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])
  return route
}

function ThemeButton({ theme, onToggle }: { theme: Theme; onToggle: () => void }) {
  return (
    <button className="theme-button" onClick={onToggle} aria-label={`Use ${theme === 'dark' ? 'light' : 'dark'} theme`}>
      <span aria-hidden="true">{theme === 'dark' ? '☼' : '◐'}</span>
    </button>
  )
}

function Header({ route, theme, onTheme }: { route: Route; theme: Theme; onTheme: () => void }) {
  return (
    <header className="topbar">
      <a className="wordmark" href="#/" aria-label="Harness evaluation overview">
        <span className="mark">iii</span>
        <span className="slash">/</span>
        <span>harness evaluation</span>
      </a>
      <nav aria-label="Presentation">
        <a className={route === 'overview' ? 'active' : ''} href="#/">overview</a>
        <a className={route === 'spec' ? 'active' : ''} href="#/spec">read the spec</a>
        <ThemeButton theme={theme} onToggle={onTheme} />
      </nav>
    </header>
  )
}

function Overview() {
  return (
    <main>
      <section className="hero section-grid">
        <div className="rail-label">architecture / 2026-07-15</div>
        <div className="hero-copy">
          <div className="status"><span /> draft technical specification</div>
          <h1>One harness.<br /><em>Two proofs.</em></h1>
          <p className="lede">
            Deterministic conformance and real-model quality answer different questions.
            The architecture keeps their boundaries—and their claims—honest.
          </p>
          <div className="hero-actions">
            <a className="primary" href="#tracks">explore the tracks <span>↓</span></a>
            <a className="secondary" href="#/spec">read canonical Markdown <span>↗</span></a>
          </div>
        </div>
        <div className="hero-diagram" aria-label="Two evaluation tracks share one public harness boundary">
          <div className="node source">public<br />harness</div>
          <div className="branch-line" />
          <div className="node branch cyan">scripted<br />router</div>
          <div className="node branch amber">real model<br />+ provider</div>
          <div className="diagram-caption">same entry · different oracle</div>
        </div>
      </section>

      <section className="constraint section-grid">
        <div className="rail-label">load-bearing constraint</div>
        <blockquote>
          A controlled model can prove a protocol.<br />
          <strong>Only a real model can reveal workflow quality.</strong>
        </blockquote>
      </section>

      <section id="tracks" className="tracks section-grid">
        <div className="rail-label">the two tracks</div>
        <div className="track-list">
          {TRACKS.map((track) => (
            <article className={`track-card ${track.color}`} key={track.label}>
              <div className="track-top">
                <span className="track-number">{track.number}</span>
                <span className="track-eyebrow">{track.eyebrow}</span>
              </div>
              <h2>{track.title}</h2>
              <p>{track.description}</p>
              <div className="track-signal">{track.signal}</div>
            </article>
          ))}
        </div>
      </section>

      <section className="flow section-grid">
        <div className="rail-label">shared discipline</div>
        <div>
          <div className="section-heading">
            <span>public path</span>
            <h2>Evidence before confidence.</h2>
          </div>
          <ol className="steps">
            {STEPS.map(([title, text], index) => (
              <li key={title}>
                <span className="step-index">{String(index + 1).padStart(2, '0')}</span>
                <div><h3>{title}</h3><p>{text}</p></div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="boundaries section-grid">
        <div className="rail-label">honest boundaries</div>
        <div>
          <div className="section-heading">
            <span>adjacent systems</span>
            <h2>Reuse ideas. Do not blur ownership.</h2>
          </div>
          <div className="boundary-table" role="table" aria-label="Adjacent system boundaries">
            {BOUNDARIES.map((row) => (
              <div className="boundary-row" role="row" key={row.system}>
                <strong role="cell">{row.system}</strong>
                <span role="cell">{row.owns}</span>
                <em role="cell">{row.relation}</em>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="contract section-grid">
        <div className="rail-label">contract discipline</div>
        <div className="contract-panel">
          <div>
            <span className="mono-label">shipped / exact</span>
            <h2>Source is the wire truth.</h2>
            <p>`harness::send`, status, transcript pagination, lifecycle payloads, router frames, and stable errors are cited from current Rust.</p>
          </div>
          <div>
            <span className="mono-label">new / proposed</span>
            <h2>Schemas are explicit.</h2>
            <p>Evaluator APIs, validator envelopes, scenarios, cassettes, evidence, and reports are strict version-1 contracts.</p>
          </div>
        </div>
      </section>

      <section className="policy section-grid">
        <div className="rail-label">release policy</div>
        <div className="policy-copy">
          <p className="policy-kicker">the green rule</p>
          <h2>Unavailable is not passing.</h2>
          <p>
            Missing infrastructure, malformed evidence, required-validator failure, browser buffer loss,
            and unexplained flakes remain non-green. Advisory findings stay visible without becoming release gates.
          </p>
          <a href="#/spec">inspect the complete failure models <span>→</span></a>
        </div>
      </section>
    </main>
  )
}

function SpecReader() {
  const [active, setActive] = useState<DocId>(docFromHash)
  const doc = SPEC_DOCS.find((item) => item.id === active) ?? SPEC_DOCS[0]
  const html = useMemo(() => renderMarkdown(doc.content), [doc.content])

  useEffect(() => {
    const update = () => setActive(docFromHash())
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])

  return (
    <main className="reader-shell">
      <aside className="reader-nav">
        <div>
          <span className="mono-label">canonical Markdown</span>
          <h1>Technical specification</h1>
        </div>
        <div className="doc-tabs" role="tablist" aria-label="Specification documents">
          {SPEC_DOCS.map((item, index) => (
            <button
              key={item.id}
              role="tab"
              aria-selected={item.id === active}
              className={item.id === active ? 'active' : ''}
              onClick={() => {
                setActive(item.id)
                window.location.hash = `#/spec?doc=${item.id}`
              }}
            >
              <span>{String(index + 1).padStart(2, '0')}</span>{item.label}
            </button>
          ))}
        </div>
        <a className="back-link" href="#/">← presentation overview</a>
      </aside>
      <article className="markdown" dangerouslySetInnerHTML={{ __html: html }} />
    </main>
  )
}

function Footer() {
  return (
    <footer>
      <span>iii workers / tech specs</span>
      <span>source of truth: Markdown</span>
    </footer>
  )
}

export default function App() {
  const route = useRoute()
  const [theme, setTheme] = useState<Theme>(() =>
    window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark',
  )

  useEffect(() => {
    document.documentElement.dataset.theme = theme
  }, [theme])

  return (
    <div className="app-shell">
      <Header route={route} theme={theme} onTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} />
      {route === 'spec' ? <SpecReader /> : <Overview />}
      <Footer />
    </div>
  )
}
