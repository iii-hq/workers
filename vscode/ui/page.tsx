import { useCallback, useEffect, useState } from 'react';
import { EmptyState, type Host, type PageRenderProps, PageHeader, PageShell, StatusPanel } from '@iii-dev/console-ui';

type Props = PageRenderProps & { host: Host };

const icon = <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden><title>VS Code</title><path d="M11.7 2.2 5.5 7 2.8 5 1.6 6.1l2.8 2.7-2.8 2.6 1.2 1.2 2.7-2 6.2 4.8 2.7-1.3V3.5l-2.7-1.3Z"/><path d="m11.7 5.3-4 3.5 4 3.5v-7Z"/></svg>;

function VscodePage({ host, onRequestClose, workingDir }: Props) {
  const [url, setUrl] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState(workingDir ?? '');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const open = useCallback(async (path: string) => {
    if (!path) return;
    setLoading(true); setError(null);
    try {
      const result = (await host.iii.trigger('vscode::start', { id: 'console-vscode', name: 'VS Code', workspace: path })) as { workspace: string; port: number };
      setWorkspace(result.workspace);
      setUrl(`http://127.0.0.1:${result.port}/`);
    } catch (cause) { setError(String(cause)); }
    finally { setLoading(false); }
  }, [host]);

  useEffect(() => { if (workingDir && (!url || workingDir !== workspace)) void open(workingDir); }, [workingDir, workspace, url, open]);

  return <PageShell>
    <PageHeader icon={icon} title="VS Code" description={workspace || 'Actual VS Code Workbench'} onClose={onRequestClose} />
    <main className="vscode-workbench-host">
      {error && <StatusPanel variant="alert" headline="VS Code Server failed" detail={error} />}
      {url ? <iframe className="vscode-workbench-frame" src={url} title="Visual Studio Code Workbench" allow="clipboard-read; clipboard-write" /> : <EmptyState title={loading ? 'Starting VS Code Server…' : 'Choose a Console working directory'} description={loading ? 'The official VS Code Server may download its matching Workbench on first launch.' : 'VS Code opens the active Console workspace automatically.'} />}
    </main>
  </PageShell>;
}

export default function setup(host: Host) {
  host.pages.register({ id: 'vscode', title: 'VS Code', render: (props) => <VscodePage host={host} {...props} /> });
  host.commands?.register('vscode', [{ id: 'open', title: 'Open VS Code', detail: 'Actual Microsoft VS Code Workbench', run: () => host.panels?.open({ pageId: 'vscode', context: {} }) }]);
}
