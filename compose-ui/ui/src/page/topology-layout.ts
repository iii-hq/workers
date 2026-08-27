export interface TopologyContainer {
  name: string
  state: string
  pid: number | null
  source: 'path' | 'package' | 'unknown' | null
  ref: string | null
  version: string | null
  ports: number[]
  startAfter: string[]
  lastError: string | null
}

export interface TopologyEngine {
  url: string | null
  host: string | null
  port: number | null
  pid: number | null
}

export interface TopologyInput {
  namespace: string | null
  file: string | null
  engine: TopologyEngine
  containers: TopologyContainer[]
}

export interface Box {
  x: number
  y: number
  w: number
  h: number
}

export interface TopologyNode extends Box {
  container: TopologyContainer
  level: number
}

export interface TopologyEdge {
  key: string
  from: string
  to: string
  start: { x: number; y: number }
  end: { x: number; y: number }
  midX: number
}

export interface TopologyLayout {
  width: number
  height: number
  engine: Box
  group: Box & { labelH: number }
  nodes: TopologyNode[]
  edges: TopologyEdge[]
}

export const TOPO = {
  pad: 20,
  engineW: 176,
  engineH: 64,
  nodeW: 208,
  nodeH: 64,
  hGap: 64,
  vGap: 12,
  groupPad: 16,
  groupLabelH: 30,
} as const

export function assignLevels(containers: TopologyContainer[]): Map<string, number> {
  const byName = new Map(containers.map((c) => [c.name, c]))
  const levels = new Map<string, number>()
  const visiting = new Set<string>()
  const level = (name: string): number => {
    const known = levels.get(name)
    if (known !== undefined) return known
    if (visiting.has(name)) return 0
    visiting.add(name)
    const container = byName.get(name)
    let depth = 0
    for (const dep of container?.startAfter ?? []) {
      if (dep === name || !byName.has(dep)) continue
      depth = Math.max(depth, level(dep) + 1)
    }
    visiting.delete(name)
    levels.set(name, depth)
    return depth
  }
  for (const c of containers) level(c.name)
  return levels
}

function byName(a: TopologyContainer, b: TopologyContainer): number {
  return a.name.localeCompare(b.name)
}

export function layoutTopology(input: TopologyInput): TopologyLayout {
  const { pad, engineW, engineH, nodeW, nodeH, hGap, vGap, groupPad, groupLabelH } = TOPO
  const containers = [...input.containers].sort(byName)
  const levels = assignLevels(containers)
  const columns = new Map<number, TopologyContainer[]>()
  for (const c of containers) {
    const lvl = levels.get(c.name) ?? 0
    const column = columns.get(lvl)
    if (column) column.push(c)
    else columns.set(lvl, [c])
  }
  const levelCount = columns.size === 0 ? 0 : Math.max(...columns.keys()) + 1
  const tallest = Math.max(0, ...[...columns.values()].map((col) => col.length))
  const columnsH = tallest === 0 ? 0 : tallest * nodeH + (tallest - 1) * vGap
  const groupInnerH = Math.max(columnsH, engineH)
  const groupX = pad + engineW + hGap
  const groupY = pad
  const groupW = levelCount === 0 ? nodeW + groupPad * 2 : groupPad * 2 + levelCount * nodeW + (levelCount - 1) * hGap
  const groupH = groupLabelH + groupPad * 2 + groupInnerH
  const contentTop = groupY + groupLabelH + groupPad

  const nodes: TopologyNode[] = []
  const positions = new Map<string, TopologyNode>()
  for (const [lvl, column] of [...columns.entries()].sort((a, b) => a[0] - b[0])) {
    const colH = column.length * nodeH + (column.length - 1) * vGap
    let y = contentTop + (groupInnerH - colH) / 2
    const x = groupX + groupPad + lvl * (nodeW + hGap)
    for (const c of column) {
      const node: TopologyNode = { container: c, level: lvl, x, y, w: nodeW, h: nodeH }
      nodes.push(node)
      positions.set(c.name, node)
      y += nodeH + vGap
    }
  }

  const engine: Box = { x: pad, y: contentTop + (groupInnerH - engineH) / 2, w: engineW, h: engineH }

  const edges: TopologyEdge[] = []
  for (const node of nodes) {
    const deps = node.container.startAfter.filter((dep) => positions.has(dep) && dep !== node.container.name)
    if (deps.length === 0) {
      edges.push({
        key: `engine→${node.container.name}`,
        from: 'engine',
        to: node.container.name,
        start: { x: engine.x + engine.w, y: engine.y + engine.h / 2 },
        end: { x: node.x, y: node.y + node.h / 2 },
        midX: engine.x + engine.w + (node.x - engine.x - engine.w) / 2,
      })
      continue
    }
    for (const dep of deps) {
      const from = positions.get(dep)
      if (!from) continue
      const startX = from.x + from.w
      edges.push({
        key: `${dep}→${node.container.name}`,
        from: dep,
        to: node.container.name,
        start: { x: startX, y: from.y + from.h / 2 },
        end: { x: node.x, y: node.y + node.h / 2 },
        midX: startX + Math.max(hGap / 2, (node.x - startX) / 2),
      })
    }
  }

  return {
    width: groupX + groupW + pad,
    height: groupY + groupH + pad,
    engine,
    group: { x: groupX, y: groupY, w: groupW, h: groupH, labelH: groupLabelH },
    nodes,
    edges,
  }
}

export function related(
  containers: TopologyContainer[],
  name: string,
): { upstream: Set<string>; downstream: Set<string> } {
  const byNameMap = new Map(containers.map((c) => [c.name, c]))
  const upstream = new Set<string>()
  const walkUp = (n: string) => {
    for (const dep of byNameMap.get(n)?.startAfter ?? []) {
      if (upstream.has(dep) || !byNameMap.has(dep)) continue
      upstream.add(dep)
      walkUp(dep)
    }
  }
  walkUp(name)
  const downstream = new Set<string>()
  const walkDown = (n: string) => {
    for (const c of containers) {
      if (!c.startAfter.includes(n) || downstream.has(c.name)) continue
      downstream.add(c.name)
      walkDown(c.name)
    }
  }
  walkDown(name)
  upstream.delete(name)
  downstream.delete(name)
  return { upstream, downstream }
}
