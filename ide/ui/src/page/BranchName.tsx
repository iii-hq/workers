/* A branch name as the chips show it: the `prefix/` quieter than the name
   after it, and the name, not the prefix, giving way to an ellipsis, so
   `fix/mot-5483-onboarding-choice-screen` still reads as which fix. */

export function splitBranch(name: string): { prefix: string; rest: string } {
  const slash = name.lastIndexOf('/')
  // A trailing slash is no prefix: the whole name is the name.
  if (slash <= 0 || slash === name.length - 1) return { prefix: '', rest: name }
  return { prefix: name.slice(0, slash + 1), rest: name.slice(slash + 1) }
}

export function BranchName({ name, className }: { name: string; className?: string }) {
  const { prefix, rest } = splitBranch(name)
  return (
    <span className={className ? `shui-branch-name ${className}` : 'shui-branch-name'}>
      {prefix ? <span className="shui-branch-name-prefix">{prefix}</span> : null}
      <span className="shui-branch-name-rest">{rest}</span>
    </span>
  )
}
