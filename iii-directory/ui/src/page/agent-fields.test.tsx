import { isValidElement, type ReactElement, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import {
  AgentFormSkeleton,
  COMPOSER_PLACEHOLDER_MAX_CHARS,
  withComposerPlaceholder,
} from './agent-fields'
import { frontmatterBody, readFrontmatterField } from './frontmatter'
// @ts-expect-error Vite exposes source files imported with the raw query.
import agentFieldsSource from './agent-fields.tsx?raw'

vi.mock('@iii-dev/console-ui', () => ({
  Skeleton: (props: Record<string, unknown>) => <span {...props} />,
}))

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
    // Two pickers (skills, preloaded functions) × two lists (selected, available).
    expect(
      classes.filter((name) => name === 'dir-ui-af-skill-list-wrap'),
    ).toHaveLength(4)
  })
})

describe('composer example field', () => {
  const draft =
    '---\nname: Helper\ndescription: "Helps."\nhidden: true\n---\nYou help.\n'

  it('writes a YAML-safe value and keeps every other key and the body', () => {
    const next = withComposerPlaceholder(draft, 'Example: add a task, then "done"')
    expect(readFrontmatterField(next, ['composer_placeholder']).value).toBe(
      'Example: add a task, then "done"',
    )
    expect(readFrontmatterField(next, ['name']).value).toBe('Helper')
    expect(readFrontmatterField(next, ['hidden']).value).toBe('true')
    expect(frontmatterBody(next)).toBe(frontmatterBody(draft))
  })

  it('drops the key when cleared, so the chat uses its generic hint', () => {
    const withValue = withComposerPlaceholder(draft, 'Example: one')
    for (const blank of ['', '   ']) {
      const cleared = withComposerPlaceholder(withValue, blank)
      expect(readFrontmatterField(cleared, ['composer_placeholder']).present).toBe(false)
      expect(readFrontmatterField(cleared, ['name']).value).toBe('Helper')
    }
  })

  it('caps input at the server limit and keeps an accessible label', () => {
    expect(COMPOSER_PLACEHOLDER_MAX_CHARS).toBe(200)
    expect(agentFieldsSource).toContain('maxLength={COMPOSER_PLACEHOLDER_MAX_CHARS}')
    expect(agentFieldsSource).toContain('>Composer example</label>')
    expect(agentFieldsSource).toContain('aria-describedby=')
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
