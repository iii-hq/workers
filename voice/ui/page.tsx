/**
 * Entry for the voice worker's injected console UI — compiled by esbuild
 * (react and @iii-dev/console-ui external) into dist/page.js, served over
 * the `console:script` trigger; styles.css ships as its own `console:style`
 * asset. `setup(host)` creates one shared dictation controller and
 * registers the voice page, its palette commands, the mic (in the composer
 * toolbar when the console has that slot, otherwise as a chat-header chip),
 * the live-transcript row above the composer, and the read-aloud turn
 * summary.
 */

import type { Host } from '@iii-dev/console-ui'
import { createVoiceSessionChip } from './src/chip'
import { createVoiceComposerAction } from './src/composer'
import { createVoiceConfigForm } from './src/configuration'
import { createDictationController } from './src/lib/dictation'
import type { ComposerCapableChat } from './src/lib/types'
import { createVoiceLiveSummary } from './src/live'
import { VoicePage } from './src/page'
import { createVoiceTurnSummary } from './src/turn-summary'

export default function setup(host: Host) {
  const controller = createDictationController(host)
  const chat = host.chat as (Host['chat'] & ComposerCapableChat) | undefined

  host.pages.register({
    id: 'voice',
    title: 'voice',
    configurationId: 'voice',
    render: (props) => <VoicePage host={host} controller={controller} {...props} />,
  })
  host.configForms.register('voice', createVoiceConfigForm(host))

  host.commands?.register('voice', [
    {
      id: 'open',
      title: 'Open voice',
      run: () => host.panels?.open({ pageId: 'voice', context: {} }),
    },
    {
      id: 'dictate',
      title: 'Start dictation',
      detail: 'Open voice and start listening',
      run: () => host.panels?.open({ pageId: 'voice', context: { action: 'dictate' } }),
    },
    {
      id: 'transcribe',
      title: 'Transcribe an audio file',
      run: () => host.panels?.open({ pageId: 'voice', context: { action: 'transcribe' } }),
    },
  ])

  if (typeof chat?.registerComposerAction === 'function') {
    chat.registerComposerAction(createVoiceComposerAction(host, controller))
  } else {
    chat?.registerSessionChip(createVoiceSessionChip(host, controller))
  }
  chat?.registerTurnSummary?.(createVoiceLiveSummary(host, controller))
  chat?.registerTurnSummary?.(createVoiceTurnSummary(host))
}
