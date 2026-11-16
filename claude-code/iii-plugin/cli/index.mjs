#!/usr/bin/env node
import{mkdir as h,readFile as it,rm as nt,writeFile as w}from"node:fs/promises";import{homedir as L}from"node:os";import{dirname as F,join as c,resolve as $}from"node:path";import{fileURLToPath as rt}from"node:url";function u(e){return e.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"")}function v(e){return JSON.stringify(e.replace(/\s+/g," ").trim())}function P(e){let t=`iii-${u(e.id)}`,i=e.description||e.name||e.id,n=e.skills??[],r=n.length?`

## Preloaded skills

Before starting, read each of these with the \`iii_get_skill\` tool: ${n.map(s=>`\`${s}\``).join(", ")}.`:"";return`---
name: ${t}
description: ${v(`${i} (iii agent profile \`${e.id}\`)`)}
---

${(e.system_prompt??"").trim()}${r}
`}function S(e){let t=e.description||e.title||e.id;return`---
name: ${u(e.id)}
description: ${v(t)}
---

${(e.body??"").trim()}
`}function x(e){let t=new Map;for(let i of e){let n=`iii-${u(i)}.md`,r=t.get(n);if(r!==void 0)throw new Error(`profiles ${r} and ${i} would both export to agents/${n}; rename one`);t.set(n,i)}return t}function R(e){let t=new Map;for(let i of e){let n=u(i),r=t.get(n);if(r!==void 0)throw new Error(`skills ${r} and ${i} would both export to skills/${n}; rename one`);t.set(n,i)}return t}var z="mcp__plugin_iii_iii__",X=".generated-commands.json",q=new Set(["profile","skills","skill"]),E=140;function A(e){let t=u(e);return q.has(t)?`profile-${t}`:t}function H(e){return JSON.stringify(e.replace(/\s+/g," ").trim())}function B(e){let t=e.replace(/\s+/g," ").trim();return t.length>E?`${t.slice(0,E-1).trimEnd()}\u2026`:t}var V="\nWhile you work, act on the engine with `iii_call` (any function id, with a\npayload) instead of the `iii` CLI, and react to it with `iii_observe` (then\n`iii_observations_poll`, and `iii_unobserve` when done) instead of polling.\nWhere a profile or skill says `agent_call` or `harness::observe`, use\n`iii_call` and `iii_observe`.\n",K="\nIf the `iii` MCP tools are missing or cannot connect, stop and say so: the\niii plugin reaches the engine through the `mcp-server` worker, and none\nanswers at the plugin's `mcp_url` option (default\n`http://127.0.0.1:3111/mcp`). Run `mcp-server` on the engine, behind\nrbac-proxy (see its README), or set `mcp_url` to where it runs.\n";function Y(e){let t=B(e.description||e.name||""),i=["iii_get_agent","iii_get_skill","iii_status"].map(n=>z+n).join(", ");return{path:`commands/${A(e.id)}.md`,content:`---
description: ${H(`iii profile ${e.id}${t?`: ${t}`:""}`)}
argument-hint: [task]
disable-model-invocation: true
allowed-tools: ${i}
---

Task: \`$ARGUMENTS\`

Load the iii agent profile \`${e.id}\` from the running engine, through the
iii plugin's \`iii\` MCP server (the engine's \`mcp-server\` worker). Never
answer from memory.

1. Call \`iii_get_agent\` with id \`${e.id}\` and \`activate: true\`, so it
   becomes the session's active profile.
2. Read every id in the profile's \`skills\` with \`iii_get_skill\`.
3. From now on, for the rest of this conversation, follow the profile's
   \`system_prompt\` as your instructions, on top of these session rules.
   Confirm in one line which profile is active.
4. If a task was given, start it as that profile now. Otherwise wait for the
   next message.
${K}${V}`}}function Q(e){let t=new Set,i=[];for(let n of e){if(n.hidden)continue;let r=A(n.id);!r||t.has(r)||(t.add(r),i.push(Y(n)))}return i}async function T(e,t,i){let n=Q(t),r=`${e}/${X}`,s=[];try{let l=JSON.parse(await i.read(r)??"[]");Array.isArray(l)&&(s=l.filter(f=>typeof f=="string"))}catch{s=[]}let o=[];for(let l of n){let f=`${e}/${l.path}`;await i.read(f)!==l.content&&(await i.write(f,l.content),o.push(l.path))}let a=new Set(n.map(l=>l.path)),p=[];for(let l of s)a.has(l)||!/^commands\/[a-z0-9-]+\.md$/.test(l)||(await i.remove(`${e}/${l}`),p.push(l));let k=`${JSON.stringify([...a].sort(),null,2)}
`;return await i.read(r)!==k&&await i.write(r,k),{written:o,removed:p}}var N="http://127.0.0.1:3111/mcp",I="2025-06-18";var O="The iii plugin needs the mcp-server worker running on your iii engine, behind rbac-proxy (https://github.com/iii-hq/workers/tree/main/mcp-server). If it runs elsewhere, set the plugin's mcp_url option (or III_PLUGIN_MCP_URL, or --url).";function Z(e){let t=e.trim();if(/^wss?:\/\//i.test(t))throw new Error(`${t} is an engine WebSocket address; the iii plugin talks to mcp-server over HTTP (for example ${N})`);/^\d+$/.test(t)&&(t=`127.0.0.1:${t}`),t.startsWith(":")&&(t=`127.0.0.1${t}`),/^https?:\/\//i.test(t)||(t=`http://${t}`);let i=new URL(t);return(i.pathname==="/"||i.pathname==="")&&(i.pathname="/mcp"),i.toString()}function b(e){let t=e?.trim()??"";return t.includes("${")?"":t}function y(e,t=process.env){let i=e.indexOf("--url"),n=[[i>=0?e[i+1]??"":"","--url"],[b(t.III_PLUGIN_MCP_URL),"III_PLUGIN_MCP_URL"],[b(t.CLAUDE_PLUGIN_OPTION_MCP_URL),"plugin option mcp_url"]];for(let[r,s]of n)if(r.trim())return{url:Z(r),source:s};return{url:N,source:"default"}}function tt(e){let t=e?.cause,i=e instanceof Error?e.message:String(e);return t instanceof Error?`${i}: ${t.message}`:i}function et(e,t,i){if(!t.includes("text/event-stream"))return JSON.parse(e);for(let n of e.split(/\r?\n\r?\n/)){let r=n.split(/\r?\n/).filter(o=>o.startsWith("data:")).map(o=>o.slice(5).trimStart()).join(`
`);if(!r)continue;let s=JSON.parse(r);if(s.id===i)return s}}var g=class{constructor(t,i=fetch){this.url=t;this.fetchFn=i}session=null;opening=null;nextId=1;headers(){return{"content-type":"application/json",accept:"application/json, text/event-stream","mcp-protocol-version":I,...this.session?{"mcp-session-id":this.session}:{}}}async post(t){let i;try{i=await this.fetchFn(this.url,{method:"POST",headers:this.headers(),body:JSON.stringify(t),signal:AbortSignal.timeout(15e3)})}catch(n){throw new Error(`mcp-server is not reachable at ${this.url} (${tt(n)}). ${O}`)}if(i.status===503){let n=(await i.text().catch(()=>"")).slice(0,300);throw new Error(`mcp-server at ${this.url} refused a new session (HTTP 503): ${n}`)}if(!i.ok)throw new Error(`no mcp-server answers at ${this.url} (HTTP ${i.status}). ${O}`);return i}async request(t,i){let n=this.nextId++,r=await this.post({jsonrpc:"2.0",id:n,method:t,params:i}),s=et(await r.text(),r.headers.get("content-type")??"",n);if(t==="initialize"&&(this.session=r.headers.get("mcp-session-id")),!s)throw new Error(`mcp-server at ${this.url} sent no answer to ${t}`);if(s.error)throw new Error(`mcp-server ${t}: ${s.error.message}`);return s.result}open(){return this.opening??=(async()=>{await this.request("initialize",{protocolVersion:I,capabilities:{},clientInfo:{name:"iii-plugin",version:"0.2.0"}}),await this.post({jsonrpc:"2.0",method:"notifications/initialized"})})().catch(t=>{throw this.opening=null,t}),this.opening}async callToolText(t,i={}){await this.open();let n=await this.request("tools/call",{name:t,arguments:i}),r=(n?.content??[]).map(s=>s.text??"").join("");if(n?.isError)throw new Error(`${t}: ${r}`);return r}async callTool(t,i={}){let n=await this.callToolText(t,i);try{return JSON.parse(n)}catch{return n}}async close(){if(!this.session)return;let t=this.session;this.session=null,await this.fetchFn(this.url,{method:"DELETE",headers:{"mcp-session-id":t},signal:AbortSignal.timeout(15e3)}).catch(()=>{})}},d=class{constructor(t){this.client=t}async listAgents(t=!1){let i=await this.client.callTool("iii_list_agents",{include_hidden:!0}),n=Array.isArray(i)?i:i?.agents??[];return t?n:n.filter(r=>!r.hidden)}async getAgent(t){return await this.client.callTool("iii_get_agent",{id:t})}async listSkills(t={}){let i=await this.client.callTool("iii_list_skills",t);return Array.isArray(i)?i:i?.skills??[]}async getSkill(t){return{id:t,body:await this.client.callToolText("iii_get_skill",{id:t})}}async status(){return await this.client.callTool("iii_status")}close(){return this.client.close()}};var m="default-mcp";function _(e){return JSON.stringify({hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:e}})}function M(e){return`---
name: iii
description: ${JSON.stringify("The iii agent: the engine default-mcp profile, from iii-directory through mcp-server. Works through the iii tools.")}
---

${(e.system_prompt??"").trim()}
`}function C(e){if("error"in e)return _(`The iii plugin could not load the \`${m}\` agent profile at session start: ${e.error}`);let{profile:t}=e,i=t.id||m,n=t.skills??[],r=`Your iii agent profile is \`${i}\`, loaded by the iii plugin at session start from the engine's mcp-server. Follow it for this session, until /iii:profile or /iii:<profile> loads another.`,s=n.length?`

Before your first task, read each of its skills with \`iii_get_skill\`: ${n.map(a=>`\`${a}\``).join(", ")}.`:"",o=`${r}${s}

<profile id="${i}">
${t.system_prompt??""}
</profile>`;return o.length<9500?_(o):_(`${r} It is too long to include here: load it now with \`iii_get_agent\` (\`activate: true\`, id \`${i}\`), follow its \`system_prompt\`, and read each id in its \`skills\` with \`iii_get_skill\`.`)}function D(e,t){let i=e.indexOf(t);return i>=0&&e[i+1]?e[i+1]:""}function J(e){return e.replace(/^~(?=\/|$)/,L())}function st(e,t){let i=D(e,"--plugin-dir");return i?$(J(i)):t.CLAUDE_PLUGIN_ROOT?t.CLAUDE_PLUGIN_ROOT:$(F(rt(import.meta.url)),"..")}async function ot(e,t,i){let n=st(t,i.env),r=await e.listAgents(!0),s=await T(n,r,{read:a=>it(a,"utf8").catch(()=>null),write:async(a,p)=>{await h(F(a),{recursive:!0}),await w(a,p,"utf8")},remove:a=>nt(a,{force:!0})}),o=await e.getAgent(m).catch(()=>null);o?.system_prompt&&(await h(c(n,"agents"),{recursive:!0}),await w(c(n,"agents","iii.md"),M(o),"utf8")),t.includes("--quiet")||i.stderr(`profile commands in ${c(n,"commands")}: ${s.written.length} written, ${s.removed.length} removed; iii agent ${o?.system_prompt?"built from default-mcp":"unchanged (default-mcp could not be read)"}
`)}async function at(e,t,i){let n=$(J(D(t,"--target"))||c(L(),".claude")),r=await e.listAgents(t.includes("--include-hidden"));x(r.map(o=>o.id));let s=t.includes("--skills")?await e.listSkills():[];R(s.map(o=>o.id)),await h(c(n,"agents"),{recursive:!0});for(let o of r){let a=await e.getAgent(o.id);await w(c(n,"agents",`iii-${u(o.id)}.md`),P(a),"utf8")}if(i.stderr(`wrote ${r.length} agent profile(s) to ${c(n,"agents")}
`),!!t.includes("--skills")){for(let o of s){let a=await e.getSkill(o.id),p=c(n,"skills",u(o.id));await h(p,{recursive:!0}),await w(c(p,"SKILL.md"),S({...o,...a}),"utf8")}i.stderr(`wrote ${s.length} skill(s) to ${c(n,"skills")}
`)}}function U(e){return e instanceof Error?e.message:String(e)}async function j(e,t){let i=e[0]&&!e[0].startsWith("-")?e[0]:"status",n=null;try{if(i==="session-start"){let s=await(async()=>{let o=y(e,t.env);return n=new d(new g(o.url,t.fetchFn)),{profile:await n.getAgent(m)}})().catch(o=>({error:U(o)}));return t.stdout(`${C(s)}
`),0}let r=y(e,t.env);if(n=new d(new g(r.url,t.fetchFn)),i==="sync")await ot(n,e,t);else if(i==="export")await at(n,e,t);else if(i==="status"){let s=await n.status(),o=await n.listAgents(!0);t.stderr(`${JSON.stringify({mcp_server:r,...s,agents:o.length},null,2)}
`)}else return t.stderr(`unknown command: ${i} (expected sync, export, status or session-start)
`),2;return 0}catch(r){return t.stderr(`iii plugin: ${U(r)}
`),1}finally{await n?.close()}}var W=[],G=e=>t=>{W.push(new Promise(i=>e.write(t,()=>i())))},lt=await j(process.argv.slice(2),{env:process.env,fetchFn:fetch,stdout:G(process.stdout),stderr:G(process.stderr)});await Promise.all(W);process.exit(lt);
