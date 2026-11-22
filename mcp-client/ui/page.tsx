/**
 * Entry for the mcp-client worker's injected console UI, compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over an
 * `ade::ui::asset` script observer (see src/ui.rs); ../styles.css ships as
 * its own `style` asset, mcp-client/styles.css.
 */

import type { Host } from '@iii-dev/console-ui'
import { McpServersPage } from './src/McpServersPage'

export default function setup(host: Host) {
  host.pages.register({
    id: 'mcp-client-servers',
    title: 'MCP client',
    render: (props) => <McpServersPage host={host} {...props} />,
  })
}
