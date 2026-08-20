import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  Badge,
  Button,
  Input,
  Markdown,
  StatusPanel,
  type Host,
} from '@iii-dev/console-ui'
import { sendAction } from './data'
import { getPath, setPath } from './bindings'
import { useLiveBindings } from './live'
import type { A2uiComponent, JsonValue, SurfaceRecord } from './types'

interface SurfaceProps {
  host: Host
  surface: SurfaceRecord
  compact?: boolean
}

const MAX_RENDER_NODES = 320

export function Surface({ host, surface, compact = false }: SurfaceProps) {
  const [model, setModel] = useState<JsonValue>(surface.data_model)
  const [revision, setRevision] = useState(surface.revision)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const pendingActionIds = useRef(new Map<string, string>())
  useEffect(() => {
    setModel(surface.data_model)
    setRevision(surface.revision)
  }, [surface.data_model, surface.revision])
  useLiveBindings(host, surface, (path, value, nextRevision) => {
    setModel((current) => setPath(current, path, value))
    setRevision(nextRevision)
  })

  const components = useMemo(
    () => new Map(surface.components.map((component) => [component.id, component])),
    [surface.components],
  )
  let renderedNodes = 0

  const renderComponent = (
    id: string,
    ancestry: ReadonlySet<string> = new Set(),
  ): ReactNode => {
    renderedNodes += 1
    if (renderedNodes > MAX_RENDER_NODES) {
      return <StatusPanel variant="alert" headline="A2UI render limit reached" />
    }
    if (ancestry.has(id)) {
      return <StatusPanel variant="alert" headline={`A2UI cycle at ${id}`} />
    }
    const component = components.get(id)
    if (!component) {
      return <StatusPanel variant="warn" headline={`Missing component ${id}`} />
    }
    const next = new Set(ancestry)
    next.add(id)
    const children = childIds(component).map((child) => (
      <div className="a2ui-child" key={child}>
        {renderComponent(child, next)}
      </div>
    ))
    const gap = enumValue(component.gap, ['sm', 'md', 'lg'], 'md')
    const align = enumValue(
      component.align,
      ['start', 'center', 'end', 'stretch'],
      'stretch',
    )

    switch (component.component) {
      case 'Column':
        return (
          <div className={`a2ui-column gap-${gap} align-${align}`}>{children}</div>
        )
      case 'Row':
        return (
          <div
            className={`a2ui-row gap-${gap} align-${align}${component.wrap === true ? ' wrap' : ''}`}
          >
            {children}
          </div>
        )
      case 'Card':
        return <section className="a2ui-card">{children}</section>
      case 'Text': {
        const text = stringValue(resolveBinding(component.text, model))
        const variant = enumValue(
          component.variant,
          ['h1', 'h2', 'body', 'caption'],
          'body',
        )
        if (variant === 'h1') return <h1 className="a2ui-text h1">{text}</h1>
        if (variant === 'h2') return <h2 className="a2ui-text h2">{text}</h2>
        return <Markdown className={`a2ui-text ${variant}`}>{text}</Markdown>
      }
      case 'Badge': {
        const variant = enumValue(
          component.variant,
          ['default', 'accent', 'warn', 'alert'],
          'default',
        )
        return <Badge variant={variant}>{stringValue(resolveBinding(component.text, model))}</Badge>
      }
      case 'Button': {
        const event = eventValue(component.action)
        return (
          <Button
            className="a2ui-button"
            variant={component.variant === 'ghost' ? 'ghost' : 'primary'}
            size="sm"
            disabled={!event || busy != null}
            onClick={async () => {
              if (!event) return
              setBusy(component.id)
              setError(null)
              const actionId = pendingActionIds.current.get(component.id) ?? crypto.randomUUID()
              pendingActionIds.current.set(component.id, actionId)
              try {
                const response = await sendAction(host, {
                  sessionId: surface.session_id,
                  surfaceId: surface.surface_id,
                  sourceComponentId: component.id,
                  actionId,
                  name: event.name,
                  context: resolveDeep(event.context ?? {}, model),
                  dataModel: surface.send_data_model ? model : undefined,
                  expectedRevision: revision,
                })
                setRevision(response.revision)
                if (response.forward_error) {
                  throw new Error(`Action was saved, but Harness delivery failed: ${response.forward_error}`)
                }
                pendingActionIds.current.delete(component.id)
              } catch (cause) {
                setError(cause instanceof Error ? cause.message : String(cause))
              } finally {
                setBusy(null)
              }
            }}
          >
            {busy === component.id ? 'sending…' : children}
          </Button>
        )
      }
      case 'TextField': {
        const path = bindingPath(component.value)
        return (
          <label className="a2ui-field">
            <span>{stringValue(component.label)}</span>
            <Input
              preserveCase
              value={stringValue(path ? getPath(model, path) : '')}
              placeholder={stringValue(component.placeholder)}
              onChange={(value) => path && setModel(setPath(model, path, value))}
            />
          </label>
        )
      }
      case 'CheckBox': {
        const path = bindingPath(component.value)
        return (
          <label className="a2ui-check">
            <input
              type="checkbox"
              checked={Boolean(path ? getPath(model, path) : false)}
              onChange={(event) =>
                path && setModel(setPath(model, path, event.currentTarget.checked))
              }
            />
            <span>{stringValue(component.label)}</span>
          </label>
        )
      }
      case 'Divider':
        return <hr className="a2ui-divider" />
      default:
        return <StatusPanel variant="warn" headline={`Unsupported ${component.component}`} />
    }
  }

  return (
    <div className={`a2ui-surface${compact ? ' compact' : ''}`}>
      {components.has('root') ? (
        renderComponent('root')
      ) : (
        <StatusPanel variant="warn" headline="Surface has no root component" />
      )}
      {error ? <div className="a2ui-action-error">{error}</div> : null}
    </div>
  )
}

function childIds(component: A2uiComponent): string[] {
  const ids: string[] = []
  if (typeof component.child === 'string') ids.push(component.child)
  if (Array.isArray(component.children)) {
    ids.push(...component.children.filter((value): value is string => typeof value === 'string'))
  }
  return ids
}

function eventValue(value: unknown): { name: string; context?: JsonValue } | null {
  if (value == null || typeof value !== 'object') return null
  const event = (value as Record<string, unknown>).event
  if (event == null || typeof event !== 'object') return null
  const record = event as Record<string, unknown>
  if (typeof record.name !== 'string') return null
  return {
    name: record.name,
    context: (record.context ?? {}) as JsonValue,
  }
}

function bindingPath(value: unknown): string | null {
  if (value == null || typeof value !== 'object') return null
  const path = (value as Record<string, unknown>).path
  return typeof path === 'string' && path.startsWith('/') ? path : null
}

function resolveBinding(value: unknown, model: JsonValue): unknown {
  const path = bindingPath(value)
  return path ? getPath(model, path) : value
}

function resolveDeep(value: JsonValue, model: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map((item) => resolveDeep(item, model))
  if (value != null && typeof value === 'object') {
    const path = bindingPath(value)
    if (path) return (getPath(model, path) ?? null) as JsonValue
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, resolveDeep(item, model)]),
    )
  }
  return value
}

function stringValue(value: unknown): string {
  if (value == null) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}

function enumValue<T extends string>(
  value: unknown,
  values: readonly T[],
  fallback: T,
): T {
  return typeof value === 'string' && values.includes(value as T)
    ? (value as T)
    : fallback
}
