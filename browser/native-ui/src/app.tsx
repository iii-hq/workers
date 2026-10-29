/**
 * The browser worker's native app for the iii app (design §8): the tabs of
 * the engine's shared Chromium, the live page of the open one, and its
 * console, network and history. Published by src/native_ui.rs as
 * browser/native.js; the host draws it as Compose (and SwiftUI later).
 */

import { defineApp } from '@iii-dev/native-ui'
import { ConsolePage } from './pages/console'
import { HistoryPage } from './pages/history'
import { NetworkPage } from './pages/network'
import { RootPage } from './pages/root'

defineApp({
  pages: {
    root: RootPage,
    console: ConsolePage,
    network: NetworkPage,
    history: HistoryPage,
  },
})
