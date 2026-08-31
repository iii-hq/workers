import { Children, isValidElement, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { SkillsDownloadView } from './DownloadView'

vi.mock('@iii-dev/console-ui', () => ({ Badge: () => null }))

describe('SkillsDownloadView', () => {
  it('keeps every written file family in the download result', () => {
    const view = SkillsDownloadView({
      input: { worker: 'resend' },
      output: {
        namespace: 'resend',
        skills_written: ['index.md'],
        system_prompts_written: ['assistant'],
        agents_written: ['reviewer'],
        source: { kind: 'registry' },
      },
    })

    expect(isValidElement(view)).toBe(true)
    if (!isValidElement<{ children: ReactNode }>(view)) return

    const writtenLists = Children.toArray(view.props.children).flatMap(
      (child) => {
        if (!isValidElement<{ label?: unknown; names?: unknown }>(child)) {
          return []
        }
        return typeof child.props.label === 'string' &&
          Array.isArray(child.props.names)
          ? [{ label: child.props.label, names: child.props.names }]
          : []
      },
    )

    expect(writtenLists).toEqual([
      { label: 'skills written', names: ['index.md'] },
      { label: 'system prompts written', names: ['assistant'] },
      { label: 'agent profiles written', names: ['reviewer'] },
    ])
  })
})
