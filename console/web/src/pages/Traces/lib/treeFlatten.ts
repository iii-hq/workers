/**
 * Iterative pre-order flatten of a node tree.
 *
 * Uses an explicit stack rather than recursion so it survives the deep
 * trees (thousands of nested spans in long-running workflows) that would
 * otherwise overflow the call stack — the same reason the span/waterfall
 * transforms were converted away from recursion.
 *
 * Generic over any node with a `children` array; emits each node before
 * its children, preserving sibling order.
 */
export function flattenPreorder<T extends { children: T[] }>(
  roots: readonly T[],
): T[] {
  const result: T[] = []
  // Seed the stack in reverse so the first root is popped first.
  const stack: T[] = []
  for (let i = roots.length - 1; i >= 0; i--) stack.push(roots[i])

  while (stack.length > 0) {
    const node = stack.pop() as T
    result.push(node)
    // Push children in reverse so they pop in their original order.
    for (let i = node.children.length - 1; i >= 0; i--) {
      stack.push(node.children[i])
    }
  }
  return result
}
