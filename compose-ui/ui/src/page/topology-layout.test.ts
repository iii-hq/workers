import { describe, expect, it } from 'vitest'
import {
  assignLevels,
  layoutTopology,
  related,
  TOPO,
  type TopologyContainer,
  type TopologyInput,
} from './topology-layout'

function container(name: string, startAfter: string[] = [], state = 'ready'): TopologyContainer {
  return {
    name,
    state,
    pid: 1,
    source: 'path',
    ref: `../${name}`,
    version: null,
    ports: [],
    startAfter,
    lastError: null,
  }
}

const input: TopologyInput = {
  namespace: 'my-project',
  file: '/proj/worker-compose.yaml',
  engine: { url: 'ws://127.0.0.1:49134', host: '127.0.0.1', port: 49134, pid: 27045 },
  containers: [
    container('harness', ['state', 'llm-router', 'console']),
    container('state'),
    container('console'),
    container('llm-router', ['state']),
    container('provider-openai', ['llm-router', 'state']),
    container('web'),
  ],
}

describe('assignLevels', () => {
  it('places a container one level after its deepest dependency', () => {
    const levels = assignLevels(input.containers)
    expect(levels.get('state')).toBe(0)
    expect(levels.get('console')).toBe(0)
    expect(levels.get('llm-router')).toBe(1)
    expect(levels.get('provider-openai')).toBe(2)
    expect(levels.get('harness')).toBe(2)
  })

  it('ignores unknown and self dependencies and survives a cycle', () => {
    const levels = assignLevels([container('a', ['b', 'ghost', 'a']), container('b', ['a'])])
    expect(levels.get('b')).toBe(1)
    expect(levels.get('a')).toBe(2)
  })
})

describe('layoutTopology', () => {
  it('lays columns left to right by level with the engine feeding level zero', () => {
    const layout = layoutTopology(input)
    const x = (name: string) => layout.nodes.find((node) => node.container.name === name)?.x ?? Number.NaN
    expect(x('state')).toBe(x('console'))
    expect(x('llm-router')).toBeGreaterThan(x('state'))
    expect(x('harness')).toBeGreaterThan(x('llm-router'))
    expect(x('harness')).toBe(x('provider-openai'))
    expect(layout.engine.x).toBe(TOPO.pad)
    expect(layout.group.x).toBeGreaterThan(layout.engine.x + layout.engine.w)
    expect(layout.width).toBeGreaterThan(layout.group.x + layout.group.w)
    expect(layout.nodes.map((node) => node.container.name)).toEqual([
      'console',
      'state',
      'web',
      'llm-router',
      'harness',
      'provider-openai',
    ])
  })

  it('draws one edge per declared dependency and engine edges for roots', () => {
    const layout = layoutTopology(input)
    const keys = layout.edges.map((edge) => edge.key).sort()
    expect(keys).toEqual(
      [
        'engine→console',
        'engine→state',
        'engine→web',
        'state→llm-router',
        'state→harness',
        'llm-router→harness',
        'console→harness',
        'llm-router→provider-openai',
        'state→provider-openai',
      ].sort(),
    )
    const edge = layout.edges.find((candidate) => candidate.key === 'state→llm-router')
    const from = layout.nodes.find((node) => node.container.name === 'state')
    const to = layout.nodes.find((node) => node.container.name === 'llm-router')
    expect(edge?.start).toEqual({ x: from!.x + from!.w, y: from!.y + from!.h / 2 })
    expect(edge?.end).toEqual({ x: to!.x, y: to!.y + to!.h / 2 })
    expect(edge!.midX).toBeGreaterThan(edge!.start.x)
    expect(edge!.midX).toBeLessThan(edge!.end.x)
  })

  it('handles an empty project', () => {
    const layout = layoutTopology({ ...input, containers: [] })
    expect(layout.nodes).toEqual([])
    expect(layout.edges).toEqual([])
    expect(layout.width).toBeGreaterThan(0)
  })
})

describe('related', () => {
  it('walks transitive upstream and downstream containers', () => {
    const { upstream, downstream } = related(input.containers, 'llm-router')
    expect([...upstream]).toEqual(['state'])
    expect([...downstream].sort()).toEqual(['harness', 'provider-openai'])
  })

  it('never includes the container itself', () => {
    const result = related([container('a', ['b']), container('b', ['a'])], 'a')
    expect(result.upstream.has('a')).toBe(false)
    expect(result.downstream.has('a')).toBe(false)
  })
})
