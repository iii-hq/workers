import { isValidElement, type ReactElement, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { AgentFormSkeleton } from './agent-fields'
// @ts-expect-error Vite exposes source files imported with the raw query.
import agentFieldsSource from './agent-fields.tsx?raw'

vi.mock('@iii-dev/console-ui', () => ({}))

function classesIn(node: ReactNode): string[] {
  if (Array.isArray(node)) return node.flatMap(classesIn)
  if (!isValidElement(node)) return []
  const element = node as ReactElement<Record<string, unknown>>
  if (typeof element.type === 'function') {
    const Component = element.type as (
      props: Record<string, unknown>,
    ) => ReactNode
    return classesIn(Component(element.props))
  }
  const props = element.props as { className?: string; children?: ReactNode }
  return [props.className ?? '', ...classesIn(props.children)]
}

describe('AgentFormSkeleton', () => {
  it('matches the visible agent form structure', () => {
    const tree = AgentFormSkeleton()
    const props = tree.props as { 'aria-label'?: string }
    const classes = classesIn(tree)

    expect(props['aria-label']).toBe('Loading agent profile')
    expect(classes).toContain('t-skel-skeleton is-pulsing')
    expect(classes).toContain('dir-ui-af-profile')
    expect(classes).toContain('dir-ui-af-model-row')
    expect(classes).toContain('dir-ui-af-prompt dir-ui-af-skeleton-prompt')
    expect(classes).toContain('dir-ui-af-skills')
    expect(
      classes.filter((name) => name === 'dir-ui-af-skill-list-wrap'),
    ).toHaveLength(2)
  })
})

describe('AgentForm loading', () => {
  it('keeps the skeleton and form mounted for the reveal transition', () => {
    const formSource = agentFieldsSource.slice(
      agentFieldsSource.indexOf('export function AgentForm'),
    )

    expect(formSource).not.toContain(
      'if (catalogsLoading) return <AgentFormSkeleton />',
    )
    expect(formSource).toContain("catalogsLoading ? '' : ' is-revealed'")
    expect(formSource).toContain('className="t-skel-content"')
    expect(formSource).toContain('inert={catalogsLoading}')
  })
})
