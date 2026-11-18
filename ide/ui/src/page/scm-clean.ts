/* What the Commit panel says when it lists nothing. The panel lists the
   changes under the opened folder (`git status -- .`), while the branch
   chip's dot marks the whole worktree (`git status`): opened on a subfolder,
   the panel said "Everything is committed" beside a dirty dot (QA
   2026-11-16). The copy names its scope whenever the two can differ. */

export interface ScmCleanCopy {
  title: string
  detail: string
  /** The branch named after `detail` ("… on <branch>"), when it is said. */
  branch: string | null
}

export function scmCleanCopy({
  branch,
  inSubfolder,
  worktreeDirty,
}: {
  branch: string | null
  /** The opened folder is below the worktree's top. */
  inSubfolder: boolean
  /** The whole worktree has uncommitted changes (the chip's dot); null unknown. */
  worktreeDirty: boolean | null
}): ScmCleanCopy {
  if (inSubfolder && worktreeDirty === true) {
    return {
      title: 'No changes in this folder',
      detail: 'Other folders in this repository have uncommitted changes. Open the repository’s top folder to see them.',
      branch: null,
    }
  }
  if (inSubfolder) {
    return { title: 'No changes', detail: 'Everything in this folder is committed', branch }
  }
  return { title: 'No changes', detail: 'Everything is committed', branch }
}

/** Two paths name the same folder, give or take a trailing slash. */
export function sameFolder(a: string, b: string): boolean {
  const trim = (path: string) => (path.length > 1 ? path.replace(/\/+$/, '') : path)
  return trim(a) === trim(b)
}
