import type { Host } from '@iii-dev/console-ui';
import { VscodePage } from './src/page/index';

export default function setup(host: Host) {
  host.pages.register({
    id: 'vscode',
    title: 'VS Code',
    render: (props) => <VscodePage host={host} {...props} />,
  });

  host.commands?.register('vscode', [
    {
      id: 'open',
      title: 'Open VS Code',
      detail: 'The VS Code Workbench for the working directory',
      keywords: ['ide', 'editor', 'workbench', 'code'],
      run: () => host.panels?.open({ pageId: 'vscode', context: {} }),
    },
  ]);
}
