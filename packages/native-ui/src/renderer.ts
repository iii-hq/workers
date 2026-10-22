/**
 * The solid-js/universal renderer over the node mirror (design §10.2). This
 * module is the `moduleName` babel-preset-solid compiles JSX against
 * (`@iii-dev/native-ui/renderer`): it exports exactly what the compiled
 * output imports, and Solid's control flow for the components the compiler
 * imports on its own when the source uses them without an import.
 */

import { createRenderer } from 'solid-js/universal'
import {
  type AnyNode,
  type Child,
  type Container,
  createNode,
  createText,
  ELEMENT,
  type ElementNode,
  insertChild,
  removeChild,
  SLOT,
  setNamedProp,
  TEXT,
  type TextNode,
} from './runtime/tree'

const renderer = createRenderer<AnyNode>({
  createElement: (type) => createNode(type),
  createTextNode: (value) => createText(String(value)),
  replaceText: (node, value) => {
    if (node.kind === TEXT) (node as TextNode).value = String(value)
  },
  isTextNode: (node) => node.kind === TEXT,
  setProperty: (node, name, value) => {
    if (node.kind === ELEMENT) setNamedProp(node as ElementNode, name, value)
  },
  insertNode: (parent, node, anchor) => {
    if (parent.kind === TEXT || node.kind === SLOT) return
    insertChild(parent as Container, node as Child, anchor as Child | undefined)
  },
  removeNode: (parent, node) => {
    if (parent.kind === TEXT || node.kind === SLOT) return
    removeChild(parent as Container, node as Child)
  },
  getParentNode: (node) => node.parent ?? undefined,
  getFirstChild: (node) => (node.kind === TEXT ? undefined : node.children[0]),
  getNextSibling: (node) => {
    const parent = node.parent
    if (!parent) return undefined
    const list = parent.children as AnyNode[]
    return list[list.indexOf(node) + 1]
  },
})

export const {
  render,
  effect,
  memo,
  createComponent,
  createElement,
  createTextNode,
  insertNode,
  insert,
  spread,
  setProp,
  mergeProps,
  use,
} = renderer

export { ErrorBoundary, For, Index, Match, Show, Suspense, SuspenseList, Switch } from 'solid-js'
