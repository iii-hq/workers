import { FileTree, type FileTreeDirectoryHandle } from '@pierre/trees'
import { describe, expect, it } from 'vitest'

import { expandedDirectoryPaths } from '../tree-expansion'

describe('expandedDirectoryPaths', () => {
  it('does not restore expanded descendants beneath a collapsed folder', () => {
    const paths = [
      'skills/',
      'skills/watch/',
      'skills/watch/SKILL.md',
      'tasklist/',
      'tasklist/tasks.md',
    ]
    const directories = ['skills', 'skills/watch', 'tasklist']
    const model = new FileTree({
      paths,
      initialExpandedPaths: directories,
    })

    const skills = model.getItem('skills') ?? model.getItem('skills/')
    expect(skills?.isDirectory()).toBe(true)
    ;(skills as FileTreeDirectoryHandle).collapse()

    const expanded = expandedDirectoryPaths(model, directories)
    expect(expanded).toEqual(['tasklist'])

    model.resetPaths([...paths, 'README.md'], {
      initialExpandedPaths: expanded.flatMap((path) => [path, `${path}/`]),
    })

    const refreshedSkills = model.getItem('skills') ?? model.getItem('skills/')
    const refreshedTasklist =
      model.getItem('tasklist') ?? model.getItem('tasklist/')
    expect((refreshedSkills as FileTreeDirectoryHandle).isExpanded()).toBe(
      false,
    )
    expect((refreshedTasklist as FileTreeDirectoryHandle).isExpanded()).toBe(
      true,
    )
  })
})
