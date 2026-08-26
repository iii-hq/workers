import { describe, expect, it } from 'vitest';
import { publicInstance, serverArgs, validateId, validateWorkspacePath } from '../src/core.js';

describe('official VS Code Server worker core', () => {
  it('validates stable ids and absolute workspaces', () => { expect(validateId('console-vscode')).toBe('console-vscode'); expect(() => validateId('../bad')).toThrow(); expect(validateWorkspacePath('/tmp/work')).toBe('/tmp/work'); expect(() => validateWorkspacePath('relative')).toThrow(/absolute/); });
  it('builds official code serve-web arguments in loopback iframe mode', () => { expect(serverArgs({bindHost:'127.0.0.1',port:18080,serverData:'/data/server',cliData:'/data/cli',workspace:'/work'})).toEqual(['--cli-data-dir','/data/cli','serve-web','--host','127.0.0.1','--port','18080','--without-connection-token','--accept-server-license-terms','--server-data-dir','/data/server','--default-folder','/work','--disable-telemetry']); });
  it('refuses cookie-free mode on a network listener', () => { expect(() => serverArgs({bindHost:'0.0.0.0',port:18080,serverData:'/data/server',cliData:'/data/cli',workspace:'/work'})).toThrow(/loopback/); });
  it('does not expose the internal server URL in instance listings', () => { const item=publicInstance({id:'x',name:'VS Code',workspace:'/work',port:18080,url:'http://127.0.0.1:18080/',pid:1,started_at:'now',status:'running',process:{} as never}); expect(item).not.toHaveProperty('url'); });
});
