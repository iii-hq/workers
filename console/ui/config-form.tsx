/**
 * Entry for the console worker's own injected UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/config-form.js and
 * served over the `console:script` trigger (see src/ui.rs). The stylesheet
 * is its own asset: styles.css ships over `console:style` as
 * console/styles.css.
 *
 * One contribution: the custom configuration form for the `console` entry —
 * the injectable-UI toggle board (src/injectable-ui-form/). Registered
 * through `host` so the loader disposes it on hot reload / disconnect.
 */

import type { ConfigFormProps, Host } from '@iii-dev/console-ui'
import { InjectableUiConfigForm } from './src/injectable-ui-form'

export default function setup(host: Host) {
  host.configForms.register('console', (props: ConfigFormProps) => (
    <InjectableUiConfigForm host={host} {...props} />
  ))
}
