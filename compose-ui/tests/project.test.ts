import { describe, expect, it } from 'vitest'
import { parseLsof, parseProject, parseWorkerRef } from '../src/project.js'

const COMPOSE = `namespace: my-project
startup_timeout: 15m
stop_timeout: 10s

engine:
  url: ws://127.0.0.1:49134

containers:
  state:
    worker: path://../state
    environment:
      RUST_LOG: info
    scripts:
      run: cargo run --locked --bin state

  llm-router:
    worker: path://../llm-router
    start_after:
      - state
    environment:
      RUST_LOG: info
      ZAI_API_KEY: "\${ZAI_API_KEY:-}"
    scripts:
      run: cargo run --locked --bin llm-router

  web:
    worker: package://api.workers.iii.dev/web
    version: "1.2.10"
`

describe('parseProject', () => {
  it('reads namespace, engine endpoint, timeouts, and declared containers', () => {
    const project = parseProject('/proj/worker-compose.yaml', COMPOSE)
    expect(project).toMatchObject({
      file: '/proj/worker-compose.yaml',
      namespace: 'my-project',
      engine_url: 'ws://127.0.0.1:49134',
      engine_host: '127.0.0.1',
      engine_port: 49134,
      startup_timeout: '15m',
      stop_timeout: '10s',
    })
    expect(project.containers.map((c) => c.name)).toEqual(['state', 'llm-router', 'web'])
    expect(project.containers[1]).toEqual({
      name: 'llm-router',
      source: 'path',
      ref: '../llm-router',
      version: null,
      start_after: ['state'],
      environment: ['RUST_LOG', 'ZAI_API_KEY'],
      run: 'cargo run --locked --bin llm-router',
    })
    expect(project.containers[2]).toMatchObject({
      source: 'package',
      ref: 'api.workers.iii.dev/web',
      version: '1.2.10',
      run: null,
    })
  })

  it('tolerates an empty or partial file', () => {
    expect(parseProject('/x', '')).toMatchObject({ namespace: null, engine_url: null, containers: [] })
    expect(parseProject('/x', 'containers:\n  a: {}\n').containers[0]).toMatchObject({ source: 'unknown', ref: '' })
  })

  it('classifies worker references', () => {
    expect(parseWorkerRef('path://../shell')).toEqual({ source: 'path', ref: '../shell' })
    expect(parseWorkerRef('package://api.workers.iii.dev/web')).toEqual({
      source: 'package',
      ref: 'api.workers.iii.dev/web',
    })
    expect(parseWorkerRef('web')).toEqual({ source: 'unknown', ref: 'web' })
  })
})

describe('parseLsof', () => {
  it('groups listening sockets by pid, dedupes, and sorts by port', () => {
    const out = [
      'p27129',
      'f10',
      'n*:3113',
      'f11',
      'n*:3113',
      'f12',
      'n127.0.0.1:49135',
      'p27045',
      'f5',
      'n[::1]:8080',
      '',
    ].join('\n')
    const ports = parseLsof(out)
    expect([...ports.keys()]).toEqual([27129, 27045])
    expect(ports.get(27129)).toEqual([
      { port: 3113, address: '*' },
      { port: 49135, address: '127.0.0.1' },
    ])
    expect(ports.get(27045)).toEqual([{ port: 8080, address: '[::1]' }])
  })

  it('keeps a pid with no listeners as an empty list', () => {
    expect(parseLsof('p1\nf3\n')).toEqual(new Map([[1, []]]))
    expect(parseLsof('')).toEqual(new Map())
  })
})
