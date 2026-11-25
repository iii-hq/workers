import { describe, expect, it } from 'vitest'
import type { ChatProject } from './backend/projects'
import {
  CHAT_NO_PROJECT_GROUP_KEY,
  capProjectRoots,
  groupByChatProject,
  groupConversationRoots,
} from './conversation-groups'
import type { ConvNode } from './conversation-tree'

function node(
  id: string,
  updatedAt: number,
  extra: Record<string, unknown> = {},
  children: ConvNode[] = [],
): ConvNode {
  return {
    conversation: {
      id,
      title: id,
      model: null,
      messages: [],
      createdAt: 0,
      updatedAt,
      ...extra,
    },
    children,
    depth: 0,
  } as unknown as ConvNode
}

function project(id: string, updated_at_ms = 0): ChatProject {
  return {
    id,
    name: id,
    color: 'blue',
    folders: [`/work/${id}`],
    created_at_ms: 0,
    updated_at_ms,
  }
}

describe('groupByChatProject (chat-projects §1.3.1)', () => {
  const web = project('acme-web')
  const billing = project('billing-api')
  const empty = project('mobile', 5)

  it('orders projects by their newest conversation and puts No project last', () => {
    const groups = groupByChatProject(
      [
        node('a', 40, { projectId: 'billing-api' }),
        node('b', 30, { projectId: 'acme-web' }),
        node('c', 50, { projectId: 'acme-web' }),
        node('d', 60),
        node('e', 70, { projectId: 'none' }),
        node('f', 80, { projectId: 'prj_deleted' }),
      ],
      [billing, web, empty],
    )
    expect(groups.map((g) => g.label)).toEqual([
      'acme-web',
      'billing-api',
      'mobile',
      'No project',
    ])
    expect(groups[0].roots.map((n) => n.conversation.id)).toEqual(['b', 'c'])
    const none = groups.at(-1)
    expect(none?.key).toBe(CHAT_NO_PROJECT_GROUP_KEY)
    expect(none?.noProject).toBe(true)
    // "none", no id and an id no project has all land under No project.
    expect(none?.roots.map((n) => n.conversation.id)).toEqual(['d', 'e', 'f'])
  })

  it('keeps an empty project as its header, with no rows', () => {
    const groups = groupByChatProject([], [empty])
    expect(groups).toHaveLength(1)
    expect(groups[0].chatProject?.id).toBe('mobile')
    expect(groups[0].roots).toEqual([])
  })

  it('leaves No project out when nothing is in it', () => {
    const groups = groupByChatProject(
      [node('a', 1, { projectId: 'acme-web' })],
      [web],
    )
    expect(groups.map((g) => g.label)).toEqual(['acme-web'])
  })

  it('is what Project grouping uses when the engine has projects', () => {
    const roots = [node('a', 1, { projectId: 'acme-web', workingDir: '/x' })]
    expect(
      groupConversationRoots(roots, 'project', 0, new Set(), [web]).map(
        (g) => g.label,
      ),
    ).toEqual(['acme-web'])
    // Without them, Project still groups by folder.
    expect(
      groupConversationRoots(roots, 'project', 0, new Set()).map((g) => g.key),
    ).toEqual(['project:/x'])
  })
})

describe('capProjectRoots: the 3 newest, then "N more"', () => {
  const roots = ['a', 'b', 'c', 'd', 'e'].map((id, i) => node(id, 10 - i))

  it('shows three and counts the rest', () => {
    const { shown, hidden } = capProjectRoots(roots, false)
    expect(shown.map((n) => n.conversation.id)).toEqual(['a', 'b', 'c'])
    expect(hidden).toBe(2)
  })

  it('shows every root once expanded ("Show less" folds it again)', () => {
    expect(capProjectRoots(roots, true)).toEqual({ shown: roots, hidden: 0 })
  })

  it('never hides the open conversation, even in a subagent', () => {
    const nested = [
      ...roots.slice(0, 4),
      node('e', 1, {}, [node('e-child', 1, { parentId: 'e' })]),
    ]
    const { shown, hidden } = capProjectRoots(nested, false, 'e-child')
    expect(shown.map((n) => n.conversation.id)).toEqual(['a', 'b', 'c', 'e'])
    expect(hidden).toBe(1)
  })

  it('has nothing to fold at three or fewer', () => {
    expect(capProjectRoots(roots.slice(0, 3), false).hidden).toBe(0)
  })
})
