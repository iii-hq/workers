/// <reference path="./vm-globals.d.ts" />
/**
 * @iii-dev/native-ui: a worker's native app for the iii app (design:
 * ide/projects/worker-native-ui/design.md). TSX components that the host
 * draws as Jetpack Compose and SwiftUI, the `iii` capability object, the
 * hooks and the page table.
 */

// The Solid primitives an app needs, from the bundle's one copy of solid-js.
// Solid's own <Switch> (control flow) is not re-exported: `Switch` here is
// the toggle; import it from solid-js under another name when needed.
export {
  batch,
  children,
  createContext,
  createEffect,
  createMemo,
  createRenderEffect,
  createResource,
  createRoot,
  createSignal,
  ErrorBoundary,
  For,
  Index,
  type JSX,
  Match,
  mergeProps,
  on,
  onCleanup,
  onMount,
  Show,
  splitProps,
  untrack,
  useContext,
} from 'solid-js'
export { type AppDefinition, defineApp, type PageComponent, type PageOptions, type PageRef } from './app'
export * from './components'
export type {
  Axis,
  BadgeVariant,
  BoxAlign,
  ButtonKind,
  ChipTone,
  CrossAlign,
  DotTone,
  ErrorCode,
  ImageFit,
  ImeAction,
  MainArrange,
  Overflow,
  RemoteMode,
  StatusVariant,
  TextAlign,
  TextRole,
  ToastKind,
  Tone,
} from './generated/catalog'
export { CATALOG_VERSION, FEATURES, LIMITS } from './generated/catalog'
export type { EventName } from './generated/events'
export type * from './generated/props'
export { useCall, useEnv, useNav, usePage, useTrigger } from './hooks'
export {
  type CallOptions,
  type Env,
  type Iii,
  IiiError,
  iii,
  type KeyboardType,
  type KvStorage,
  type MenuAction,
  type Nav,
  type ShareFile,
  type ShareText,
  type Ui,
} from './iii'
export { createList, List, type ListProps, type ListSource } from './list'
export type { Binding, Bound, IconName, JsonObject, JsonValue, NodeRef, Padding, Prop, Unsubscribe } from './types'
export { parseUrl } from './url'
export { SDK_VERSION } from './version'
