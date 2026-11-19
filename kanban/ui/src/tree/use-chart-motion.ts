import { type RefObject, useLayoutEffect, useRef } from 'react'
import { type Axis, advance, axis, settled } from './spring'
import { type Camera, easeOut } from './use-camera'

/**
 * How the Tree moves when it changes under you (a session spawns, a ticket arrives, a fold opens):
 *
 * - Every card glides from where it was drawn to its new place on a critically damped spring (spring.ts): the
 *   card is offset back to where it was and pulled to its place. A second move mid-flight adds to the offset and
 *   keeps the velocity, so a tree that changes every few hundred milliseconds bends its motion, never restarts it.
 * - The connectors are one SVG layer under the cards, routed from where the cards are *drawn* each frame while
 *   anything moves — the lines bend with the cards instead of snapping to the final layout.
 * - A new edge draws out of its parent; the card it leads to grows in as the line arrives (CSS, tree.css).
 * - The camera keeps the right thing still: while Fit holds it glides to the new fit, after a fold it keeps that
 *   card where it was, otherwise the root. Same curve and length as the cards' glide, so a card held still is still.
 *
 * Nothing glides or draws on the first render of a chart (or of another project): that is not a change.
 * prefers-reduced-motion: no glides and no drawing; the cards keep their fade.
 */

/** A new wire draws out of its parent in this long. */
const DRAW_MS = 240
/** The stacked spine runs this far in from a card's left edge, and elbows in this far below a card's top. */
const STACK_X = 25
const STACK_ELBOW = 25
const RADIUS = 10
const SVG = 'http://www.w3.org/2000/svg'

type Box = { x: number; y: number; w: number; h: number }
type Kind = 'single' | 'row' | 'stack'
type Wire = { path: SVGPathElement; parent: string; child: string; kind: Kind; born: number }

const reducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true

/** A card's place in the stage, transforms ignored (its layout, not where a glide is drawing it). */
function layoutOf(el: HTMLElement, stage: HTMLElement): Box {
  let x = 0
  let y = 0
  let node: HTMLElement | null = el
  while (node && node !== stage) {
    x += node.offsetLeft
    y += node.offsetTop
    node = node.offsetParent as HTMLElement | null
  }
  return { x, y, w: el.offsetWidth, h: el.offsetHeight }
}

function route(kind: Kind, p: Box, c: Box): string {
  if (kind === 'stack') {
    const ax = p.x + STACK_X
    const ay = p.y + p.h
    const ex = c.x
    const ey = c.y + STACK_ELBOW
    const r = Math.max(0, Math.min(RADIUS, (ey - ay) / 2, (ex - ax) / 2))
    return `M${ax} ${ay}V${ey - r}Q${ax} ${ey} ${ax + r} ${ey}H${ex}`
  }
  const ax = p.x + p.w / 2
  const ay = p.y + p.h
  const cx = c.x + c.w / 2
  const cy = c.y
  if (Math.abs(cx - ax) < 1) return `M${ax} ${ay}V${cy}`
  const bus = ay + (cy - ay) / 2
  const dir = Math.sign(cx - ax)
  const r = Math.max(0, Math.min(RADIUS, Math.abs(cx - ax) / 2, (cy - ay) / 4))
  return `M${ax} ${ay}V${bus - r}Q${ax} ${bus} ${ax + dir * r} ${bus}H${cx - dir * r}Q${cx} ${bus} ${cx} ${bus + r}V${cy}`
}

export function useChartMotion({
  camera,
  chart,
  anchor,
  chartKey,
}: {
  camera: Camera
  chart: RefObject<HTMLDivElement | null>
  /** The card a fold was asked on: kept still through the re-layout (read once, then cleared). */
  anchor: { current: string | null }
  /** Changes when another chart is shown (another project): the next render is a baseline. */
  chartKey: string | null
}) {
  const boxes = useRef(new Map<string, Box>())
  /** Cards still on their way: how far each is drawn from its place, per axis, and how fast it is closing. */
  const springs = useRef(new Map<string, { x: Axis; y: Axis }>())
  const lastFrame = useRef(0)
  const live = useRef(new Map<string, Wire>())
  const rings = useRef(new Map<string, SVGCircleElement>())
  const shown = useRef<{ key: string | null; stage: HTMLElement | null }>({ key: null, stage: null })
  const stageSize = useRef({ w: 0, h: 0 })
  const loop = useRef(0)
  /** The cards and their parents as last laid out: what each frame routes. */
  const cards = useRef(new Map<string, { el: HTMLElement; kids: Kind | null }>())

  useLayoutEffect(() => {
    const stage = camera.stage
    const root = chart.current
    if (!stage || !root) return
    // The wire layer: one SVG at the back of the stage, made here (it is drawn imperatively, never by React).
    let svg = stage.querySelector<SVGSVGElement>(':scope > svg.kb-chart__wires')
    if (!svg) {
      svg = document.createElementNS(SVG, 'svg')
      svg.setAttribute('class', 'kb-chart__wires')
      svg.setAttribute('aria-hidden', 'true')
      stage.prepend(svg)
    }
    const layer = svg
    const reduced = reducedMotion()
    const baseline = shown.current.key !== chartKey || shown.current.stage !== stage
    if (baseline) {
      shown.current = { key: chartKey, stage }
      boxes.current = new Map()
      for (const id of springs.current.keys()) {
        const el = cards.current.get(id)?.el
        if (el) el.style.transform = ''
      }
      springs.current = new Map()
      for (const wire of live.current.values()) wire.path.remove()
      live.current = new Map()
      for (const ring of rings.current.values()) ring.remove()
      rings.current = new Map()
    }

    // ── read the new layout ─────────────────────────────────────────────────
    const next = new Map<string, Box>()
    const nextCards = new Map<string, { el: HTMLElement; kids: Kind | null }>()
    const edges: { parent: string; child: string; kind: Kind }[] = []
    /** Cards with an agent at work at or under them: the wire into each is drawn live. */
    const liveIds = new Set<string>()
    for (const item of root.querySelectorAll<HTMLElement>('.kb-chart__item')) {
      const id = item.dataset.nodeId
      const card = item.firstElementChild as HTMLElement | null
      if (!id || !card) continue
      next.set(id, layoutOf(card, stage))
      if (item.dataset.live === 'true') liveIds.add(id)
      nextCards.set(id, { el: card, kids: (item.dataset.kids as Kind | undefined) ?? null })
      const parent = item.parentElement?.closest<HTMLElement>('.kb-chart__item')
      const parentId = parent?.dataset.nodeId
      const kind = parent?.dataset.kids as Kind | undefined
      if (parentId && kind) edges.push({ parent: parentId, child: id, kind })
    }

    // ── what to keep still ──────────────────────────────────────────────────
    // While Fit holds, the camera glides to the new fit. Otherwise one card stays exactly where it is on screen —
    // the one a fold was asked on, else the root — by moving the camera now, before this frame paints; every
    // other card then glides from where it was drawn. (Two opposing animations, camera and cards, never land in
    // the same frame; one instant correction and the cards' own glide always do.)
    const anchorId = anchor.current
    anchor.current = null
    const firstId = root.querySelector<HTMLElement>('.kb-chart__item')?.dataset.nodeId ?? null
    const following = camera.isFollowing()
    let comp = { x: 0, y: 0 }
    if (!baseline && !following) {
      const held = (anchorId && next.has(anchorId) && boxes.current.has(anchorId) ? anchorId : null) ?? firstId
      const before = held ? boxes.current.get(held) : undefined
      const now = held ? next.get(held) : undefined
      if (before && now && (Math.abs(now.x - before.x) >= 0.5 || Math.abs(now.y - before.y) >= 0.5)) {
        const s = camera.scaleNow()
        const made = camera.shiftNow(-(now.x - before.x) * s, -(now.y - before.y) * s)
        comp = { x: -made.dx / s, y: -made.dy / s }
      }
    }

    // ── every card that moved: offset back to where it was drawn, the spring takes it home ──────────────
    for (const [id, now] of next) {
      const before = boxes.current.get(id)
      if (!before) continue
      const dx = before.x - now.x + comp.x
      const dy = before.y - now.y + comp.y
      // Its place did not change: a running spring carries on untouched (never restarted by a re-render).
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue
      if (reduced) continue
      const spring = springs.current.get(id) ?? { x: axis(), y: axis() }
      spring.x.x += dx
      spring.y.x += dy
      springs.current.set(id, spring)
    }
    for (const id of springs.current.keys()) if (!next.has(id)) springs.current.delete(id)
    // Drawn where the springs say before this frame paints (the new layout must never flash).
    for (const [id, spring] of springs.current) {
      const el = nextCards.get(id)?.el
      if (el) el.style.transform = `translate(${spring.x.x}px, ${spring.y.x}px)`
    }
    boxes.current = next
    cards.current = nextCards

    const size = { w: stage.offsetWidth, h: stage.offsetHeight }
    const resized = size.w !== stageSize.current.w || size.h !== stageSize.current.h
    stageSize.current = size
    if (!baseline && following && resized) camera.refit()

    // ── wires: one path per edge, new ones drawing out of their parent ─────
    const seen = new Set<string>()
    for (const edge of edges) {
      const key = `${edge.parent}>${edge.child}`
      seen.add(key)
      const wire = live.current.get(key)
      if (wire) {
        wire.kind = edge.kind
        continue
      }
      const path = document.createElementNS(SVG, 'path')
      path.setAttribute('class', 'kb-chart__wire')
      layer.appendChild(path)
      live.current.set(key, { path, ...edge, born: baseline || reduced ? 0 : performance.now() })
    }
    for (const [key, wire] of live.current) {
      if (seen.has(key)) {
        // Live: the work at its end is in flight (tree.css draws it flowing; reduced motion, just the colour).
        wire.path.toggleAttribute('data-live', liveIds.has(wire.child))
        continue
      }
      wire.path.remove()
      live.current.delete(key)
    }
    const parents = new Set(edges.map((edge) => edge.parent))
    for (const id of parents) {
      if (rings.current.has(id)) continue
      const ring = document.createElementNS(SVG, 'circle')
      ring.setAttribute('class', 'kb-chart__ring')
      ring.setAttribute('r', '3.5')
      layer.appendChild(ring)
      rings.current.set(id, ring)
    }
    for (const [id, ring] of rings.current) {
      if (parents.has(id)) continue
      ring.remove()
      rings.current.delete(id)
    }

    // ── draw now (before paint), and every frame while something moves ─────
    const frame = (dt: number): boolean => {
      const scale = camera.scaleNow()
      for (const [id, spring] of springs.current) {
        advance(spring.x, dt)
        advance(spring.y, dt)
        const el = cards.current.get(id)?.el
        if (settled(spring.x, scale) && settled(spring.y, scale)) {
          springs.current.delete(id)
          if (el) el.style.transform = ''
        } else if (el) {
          el.style.transform = `translate(${spring.x.x}px, ${spring.y.x}px)`
        }
      }
      layer.setAttribute('width', String(stage.offsetWidth))
      layer.setAttribute('height', String(stage.offsetHeight))
      const drawn = new Map<string, Box>()
      const at = (id: string): Box | undefined => {
        const cached = drawn.get(id)
        if (cached) return cached
        const box = boxes.current.get(id)
        const el = cards.current.get(id)?.el
        if (!box || !el) return undefined
        const spring = springs.current.get(id)
        const offset = spring ? { x: spring.x.x, y: spring.y.x } : { x: 0, y: 0 }
        const result = { ...box, x: box.x + offset.x, y: box.y + offset.y }
        drawn.set(id, result)
        return result
      }
      const now = performance.now()
      let busy = springs.current.size > 0
      for (const wire of live.current.values()) {
        const p = at(wire.parent)
        const c = at(wire.child)
        if (!p || !c) continue
        wire.path.setAttribute('d', route(wire.kind, p, c))
        if (!wire.born) continue
        const t = (now - wire.born) / DRAW_MS
        if (t >= 1) {
          wire.born = 0
          wire.path.style.strokeDasharray = ''
          wire.path.style.strokeDashoffset = ''
          continue
        }
        const length = wire.path.getTotalLength()
        wire.path.style.strokeDasharray = `${length} ${length}`
        wire.path.style.strokeDashoffset = String(length * (1 - easeOut(Math.max(0, t))))
        busy = true
      }
      for (const [id, ring] of rings.current) {
        const p = at(id)
        const kids = cards.current.get(id)?.kids
        if (!p) continue
        ring.setAttribute('cx', String(kids === 'stack' ? p.x + STACK_X : p.x + p.w / 2))
        ring.setAttribute('cy', String(p.y + p.h))
      }
      return busy
    }
    cancelAnimationFrame(loop.current)
    lastFrame.current = performance.now()
    if (frame(0)) {
      const tick = (now: number) => {
        const dt = (now - lastFrame.current) / 1000
        lastFrame.current = now
        loop.current = frame(dt) ? requestAnimationFrame(tick) : 0
      }
      loop.current = requestAnimationFrame(tick)
    }
  })

  useLayoutEffect(() => () => cancelAnimationFrame(loop.current), [])
}
