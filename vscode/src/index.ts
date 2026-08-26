import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdir, rm, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:net';
import { registerWorker } from 'iii-sdk';
import { uiPage, uiStyles } from 'virtual:vscode-ui';
import { type Instance, publicInstance, schema, serverArgs, validateId, validateWorkspacePath } from './core.js';

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134', { workerName: 'vscode', workerDescription: 'Actual VS Code Workbench as a standalone injectable Console IDE worker.' });
const binary = process.env.VSCODE_SERVER_BIN ?? 'code';
const dataDir = process.env.VSCODE_DATA_DIR ?? join(homedir(), '.iii', 'vscode');
const bindHost = process.env.VSCODE_BIND_HOST ?? '127.0.0.1';
const portMin = Number(process.env.VSCODE_PORT_MIN ?? 18080);
const portMax = Number(process.env.VSCODE_PORT_MAX ?? 18180);
const instances = new Map<string, Instance>();
const fields = { id:{type:'string'}, name:{type:'string'}, workspace:{type:'string'}, port:{type:'integer'}, pid:{type:['integer','null']}, started_at:{type:'string'}, status:{type:'string'}, exit_code:{type:['integer','null']} };

async function freePort() { for (let port=portMin; port<=portMax; port++) { const available = await new Promise<boolean>((done) => { const s=createServer(); s.once('error',()=>done(false)); s.listen(port,bindHost,()=>s.close(()=>done(true))); }); if (available) return port; } throw new Error('no VS Code port available'); }
async function stop(instance: Instance) {
  if (instance.pid === null) return;
  const signalGroup = (signal: NodeJS.Signals) => {
    try { process.kill(-instance.pid!, signal); } catch { instance.process.kill(signal); }
  };
  if (instance.process.exitCode === null && instance.process.signalCode === null) signalGroup('SIGTERM');
  await new Promise<void>((done) => {
    const timer = setTimeout(() => { signalGroup('SIGKILL'); done(); }, 5000);
    if (instance.process.exitCode !== null || instance.process.signalCode !== null) { clearTimeout(timer); done(); }
    else instance.process.once('exit', () => { clearTimeout(timer); done(); });
  });
  instance.status = 'stopped';
}
async function ready(instance: Instance, timeoutMs = 120_000) { const deadline=Date.now()+timeoutMs; while(Date.now()<deadline){ if(instance.process.exitCode!==null) throw new Error('VS Code exited before becoming ready'); try { const r=await fetch(instance.url,{redirect:'manual'}); if(r.status===200||(r.status>=300&&r.status<400)){instance.status='running';return;} } catch {} await new Promise(r=>setTimeout(r,100)); } instance.status='failed'; await stop(instance); throw new Error('VS Code did not become ready within the configured timeout'); }
function get(id:string){const item=instances.get(id);if(!item)throw new Error(`Unknown VS Code workspace: ${id}`);return item;}

iii.registerFunction('vscode::start', async (input:{id?:string;name?:string;workspace:string})=>{ const workspace=validateWorkspacePath(input.workspace); if(!(await stat(workspace)).isDirectory())throw new Error('workspace must be a directory'); const id=validateId(input.id??`ide-${randomBytes(4).toString('hex')}`); const existing=instances.get(id); if(existing?.status==='running'&&existing.workspace===workspace)return publicInstance(existing); if(existing)await stop(existing); if(spawnSync(binary,['--version'],{stdio:'ignore'}).status!==0)throw new Error(`Official VS Code CLI executable not available: ${binary}`); const port=await freePort(); const root=join(dataDir,id); await mkdir(join(root,'server-data'),{recursive:true}); await mkdir(join(root,'cli-data'),{recursive:true}); const child=spawn(binary,serverArgs({bindHost,port,serverData:join(root,'server-data'),cliData:join(root,'cli-data'),workspace}),{stdio:['ignore','pipe','pipe'],detached:true}); const item:Instance={id,name:input.name?.trim()||'VS Code',workspace,port,url:`http://${bindHost}:${port}/`,pid:child.pid??null,started_at:new Date().toISOString(),status:'starting',process:child}; instances.set(id,item); child.once('exit',(code)=>{item.exit_code=code;item.status=code===0?'stopped':'failed';}); child.stderr?.on('data',(x)=>process.stderr.write(`[vscode:${id}] ${x}`)); await ready(item); return publicInstance(item); }, {description:'Start the actual VS Code Workbench for one absolute workspace.',request_format:schema({id:{type:'string'},name:{type:'string'},workspace:{type:'string'}},['workspace']),response_format:schema(fields,['id','workspace','status'])});
iii.registerFunction('vscode::instances::list',async()=>({instances:[...instances.values()].map(publicInstance)}),{description:'List worker-owned VS Code Workbench processes.',request_format:schema({}),response_format:schema({instances:{type:'array',items:schema(fields)}} ,['instances'])});
iii.registerFunction('vscode::stop',async(input:{id:string})=>{const item=get(input.id);await stop(item);return publicInstance(item);},{description:'Stop a VS Code Workbench process.',request_format:schema({id:{type:'string'}},['id']),response_format:schema(fields,['id','status'])});
iii.registerFunction('vscode::delete',async(input:{id:string;delete_profile?:boolean})=>{const item=get(input.id);await stop(item);instances.delete(input.id);if(input.delete_profile)await rm(join(dataDir,input.id),{recursive:true,force:true});return{deleted:true};},{description:'Remove the VS Code process and optionally its isolated profile.',request_format:schema({id:{type:'string'},delete_profile:{type:'boolean'}},['id']),response_format:schema({deleted:{type:'boolean'}},['deleted'])});
const assets=new Map([['vscode/page.js',{content:uiPage,type:'text/javascript'}],['vscode/styles.css',{content:uiStyles,type:'text/css'}]]); iii.registerFunction('vscode::ui-content',async(input:{path:string})=>{const a=assets.get(input.path);if(!a)throw new Error('unknown asset');return{content:a.content,content_type:a.type};},{description:'Serve the native injectable VS Code Console page.',request_format:schema({path:{type:'string'}},['path']),response_format:schema({content:{type:'string'},content_type:{type:'string'}},['content'])}); iii.registerTrigger({type:'console:script',function_id:'vscode::ui-content',config:{path:'vscode/page.js'}});iii.registerTrigger({type:'console:style',function_id:'vscode::ui-content',config:{path:'vscode/styles.css'}});
async function shutdown(){await Promise.all([...instances.values()].map(stop));await iii.shutdown();} process.on('SIGTERM',()=>void shutdown());process.on('SIGINT',()=>void shutdown());
