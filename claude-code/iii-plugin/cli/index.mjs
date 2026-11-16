#!/usr/bin/env node
import{mkdir as _,readFile as F,rm as nt,writeFile as $}from"node:fs/promises";import{homedir as D}from"node:os";import{dirname as j,join as u,resolve as k}from"node:path";import{fileURLToPath as rt}from"node:url";function c(e){return e.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"")}function P(e){return JSON.stringify(e.replace(/\s+/g," ").trim())}function x(e){let t=`iii-${c(e.id)}`,i=e.description||e.name||e.id,n=e.skills??[],r=n.length?`

## Preloaded skills

Before starting, read each of these with the \`iii_get_skill\` tool: ${n.map(o=>`\`${o}\``).join(", ")}.`:"";return`---
name: ${t}
description: ${P(`${i} (iii agent profile \`${e.id}\`)`)}
---

${(e.system_prompt??"").trim()}${r}
`}function S(e){let t=e.description||e.title||e.id;return`---
name: ${c(e.id)}
description: ${P(t)}
---

${(e.body??"").trim()}
`}function R(e){let t=new Map;for(let i of e){let n=`iii-${c(i)}.md`,r=t.get(n);if(r!==void 0)throw new Error(`profiles ${r} and ${i} would both export to agents/${n}; rename one`);t.set(n,i)}return t}function E(e){let t=new Map;for(let i of e){let n=c(i),r=t.get(n);if(r!==void 0)throw new Error(`skills ${r} and ${i} would both export to skills/${n}; rename one`);t.set(n,i)}return t}var X="mcp__plugin_iii_iii__",q=".generated-commands.json",H=new Set(["profile","skills","skill"]),A=140;function T(e){let t=c(e);return H.has(t)?`profile-${t}`:t}function B(e){return JSON.stringify(e.replace(/\s+/g," ").trim())}function V(e){let t=e.replace(/\s+/g," ").trim();return t.length>A?`${t.slice(0,A-1).trimEnd()}\u2026`:t}var K="\nWhile you work, act on the engine with `iii_call` (any function id, with a\npayload) instead of the `iii` CLI, and react to it with `iii_observe` (then\n`iii_observations_poll`, and `iii_unobserve` when done) instead of polling.\nWhere a profile or skill says `agent_call` or `harness::observe`, use\n`iii_call` and `iii_observe`.\n",Y="\nIf the `iii` MCP tools are missing or cannot connect, stop and say so: the\niii plugin reaches the engine through the `mcp-server` worker, and none\nanswers at the plugin's `mcp_url` option (default\n`http://127.0.0.1:3111/mcp`). Run `mcp-server` on the engine, behind\nrbac-proxy (see its README), or set `mcp_url` to where it runs.\n";function Q(e){let t=V(e.description||e.name||""),i=["iii_get_agent","iii_get_skill","iii_status"].map(n=>X+n).join(", ");return{path:`commands/${T(e.id)}.md`,content:`---
description: ${B(`iii profile ${e.id}${t?`: ${t}`:""}`)}
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
${Y}${K}`}}function Z(e){let t=new Set,i=[];for(let n of e){if(n.hidden)continue;let r=T(n.id);!r||t.has(r)||(t.add(r),i.push(Q(n)))}return i}async function I(e,t,i){let n=Z(t),r=`${e}/${q}`,o=[];try{let a=JSON.parse(await i.read(r)??"[]");Array.isArray(a)&&(o=a.filter(m=>typeof m=="string"))}catch{o=[]}let s=[];for(let a of n){let m=`${e}/${a.path}`;await i.read(m)!==a.content&&(await i.write(m,a.content),s.push(a.path))}let l=new Set(n.map(a=>a.path)),p=[];for(let a of o)l.has(a)||!/^commands\/[a-z0-9-]+\.md$/.test(a)||(await i.remove(`${e}/${a}`),p.push(a));let v=`${JSON.stringify([...l].sort(),null,2)}
`;return await i.read(r)!==v&&await i.write(r,v),{written:s,removed:p}}var N="http://127.0.0.1:3111/mcp",O="2025-06-18";var b="The iii plugin needs the mcp-server worker running on your iii engine, behind rbac-proxy (https://github.com/iii-hq/workers/tree/main/mcp-server). If it runs elsewhere, set the plugin's mcp_url option (or III_PLUGIN_MCP_URL, or --url).";function tt(e){let t=e.trim();if(/^wss?:\/\//i.test(t))throw new Error(`${t} is an engine WebSocket address; the iii plugin talks to mcp-server over HTTP (for example ${N})`);/^\d+$/.test(t)&&(t=`127.0.0.1:${t}`),t.startsWith(":")&&(t=`127.0.0.1${t}`),/^https?:\/\//i.test(t)||(t=`http://${t}`);let i=new URL(t);return(i.pathname==="/"||i.pathname==="")&&(i.pathname="/mcp"),i.toString()}function M(e){let t=e?.trim()??"";return t.includes("${")?"":t}function w(e,t=process.env){let i=e.indexOf("--url"),n=[[i>=0?e[i+1]??"":"","--url"],[M(t.III_PLUGIN_MCP_URL),"III_PLUGIN_MCP_URL"],[M(t.CLAUDE_PLUGIN_OPTION_MCP_URL),"plugin option mcp_url"]];for(let[r,o]of n)if(r.trim())return{url:tt(r),source:o};return{url:N,source:"default"}}function et(e){let t=e?.cause,i=e instanceof Error?e.message:String(e);return t instanceof Error?`${i}: ${t.message}`:i}function it(e,t,i){if(!t.includes("text/event-stream"))return JSON.parse(e);for(let n of e.split(/\r?\n\r?\n/)){let r=n.split(/\r?\n/).filter(s=>s.startsWith("data:")).map(s=>s.slice(5).trimStart()).join(`
`);if(!r)continue;let o=JSON.parse(r);if(o.id===i)return o}}var g=class{constructor(t,i=fetch){this.url=t;this.fetchFn=i}session=null;opening=null;nextId=1;headers(){return{"content-type":"application/json",accept:"application/json, text/event-stream","mcp-protocol-version":O,...this.session?{"mcp-session-id":this.session}:{}}}async post(t){let i;try{i=await this.fetchFn(this.url,{method:"POST",headers:this.headers(),body:JSON.stringify(t),signal:AbortSignal.timeout(15e3)})}catch(n){throw new Error(`mcp-server is not reachable at ${this.url} (${et(n)}). ${b}`)}if(i.status===503){let n=(await i.text().catch(()=>"")).slice(0,300);throw new Error(`mcp-server at ${this.url} refused a new session (HTTP 503): ${n}`)}if(!i.ok)throw new Error(`no mcp-server answers at ${this.url} (HTTP ${i.status}). ${b}`);return i}async request(t,i){let n=this.nextId++,r=await this.post({jsonrpc:"2.0",id:n,method:t,params:i}),o=it(await r.text(),r.headers.get("content-type")??"",n);if(t==="initialize"&&(this.session=r.headers.get("mcp-session-id")),!o)throw new Error(`mcp-server at ${this.url} sent no answer to ${t}`);if(o.error)throw new Error(`mcp-server ${t}: ${o.error.message}`);return o.result}open(){return this.opening??=(async()=>{await this.request("initialize",{protocolVersion:O,capabilities:{},clientInfo:{name:"iii-plugin",version:"0.2.0"}}),await this.post({jsonrpc:"2.0",method:"notifications/initialized"})})().catch(t=>{throw this.opening=null,t}),this.opening}async callToolText(t,i={}){await this.open();let n=await this.request("tools/call",{name:t,arguments:i}),r=(n?.content??[]).map(o=>o.text??"").join("");if(n?.isError)throw new Error(`${t}: ${r}`);return r}async callTool(t,i={}){let n=await this.callToolText(t,i);try{return JSON.parse(n)}catch{return n}}async close(){if(!this.session)return;let t=this.session;this.session=null,await this.fetchFn(this.url,{method:"DELETE",headers:{"mcp-session-id":t},signal:AbortSignal.timeout(15e3)}).catch(()=>{})}},d=class{constructor(t){this.client=t}async listAgents(t=!1){let i=await this.client.callTool("iii_list_agents",{include_hidden:!0}),n=Array.isArray(i)?i:i?.agents??[];return t?n:n.filter(r=>!r.hidden)}async getAgent(t){return await this.client.callTool("iii_get_agent",{id:t})}async listSkills(t={}){let i=await this.client.callTool("iii_list_skills",t);return Array.isArray(i)?i:i?.skills??[]}async getSkill(t){return{id:t,body:await this.client.callToolText("iii_get_skill",{id:t})}}async status(){return await this.client.callTool("iii_status")}close(){return this.client.close()}};var h="default-mcp";function f(e){return JSON.stringify({hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:e}})}function C(e){return e.replace(/^---\n[\s\S]*?\n---\n/,"").trim()}function U(e){return f(e.trim())}function L(e){if("error"in e)return f(`The iii plugin could not load the \`${h}\` agent profile at session start: ${e.error}`);let{profile:t}=e,i=t.id||h,n=t.skills??[],r=`Your iii agent profile is \`${i}\`, loaded by the iii plugin at session start from the engine's mcp-server. Follow it for this session, until /iii:profile or /iii:<profile> loads another.`,o=n.length?`

Before your first task, read each of its skills with \`iii_get_skill\`: ${n.map(l=>`\`${l}\``).join(", ")}.`:"",s=`${r}${o}

<profile id="${i}">
${t.system_prompt??""}
</profile>`;return s.length<9500?f(s):f(`${r} It is too long to include here: load it now with \`iii_get_agent\` (\`activate: true\`, id \`${i}\`), follow its \`system_prompt\`, and read each id in its \`skills\` with \`iii_get_skill\`.`)}function J(e,t){let i=e.indexOf(t);return i>=0&&e[i+1]?e[i+1]:""}function G(e){return e.replace(/^~(?=\/|$)/,D())}function z(e,t){let i=J(e,"--plugin-dir");return i?k(G(i)):t.CLAUDE_PLUGIN_ROOT?t.CLAUDE_PLUGIN_ROOT:k(j(rt(import.meta.url)),"..")}async function st(e,t,i){let n=z(t,i.env),r=await e.listAgents(!0),o=await I(n,r,{read:s=>F(s,"utf8").catch(()=>null),write:async(s,l)=>{await _(j(s),{recursive:!0}),await $(s,l,"utf8")},remove:s=>nt(s,{force:!0})});t.includes("--quiet")||i.stderr(`profile commands in ${u(n,"commands")}: ${o.written.length} written, ${o.removed.length} removed
`)}async function ot(e,t,i){let n=k(G(J(t,"--target"))||u(D(),".claude")),r=await e.listAgents(t.includes("--include-hidden"));R(r.map(s=>s.id));let o=t.includes("--skills")?await e.listSkills():[];E(o.map(s=>s.id)),await _(u(n,"agents"),{recursive:!0});for(let s of r){let l=await e.getAgent(s.id);await $(u(n,"agents",`iii-${c(s.id)}.md`),x(l),"utf8")}if(i.stderr(`wrote ${r.length} agent profile(s) to ${u(n,"agents")}
`),!!t.includes("--skills")){for(let s of o){let l=await e.getSkill(s.id),p=u(n,"skills",c(s.id));await _(p,{recursive:!0}),await $(u(p,"SKILL.md"),S({...s,...l}),"utf8")}i.stderr(`wrote ${o.length} skill(s) to ${u(n,"skills")}
`)}}function y(e){return e instanceof Error?e.message:String(e)}async function W(e,t){let i=e[0]&&!e[0].startsWith("-")?e[0]:"status";if(i==="mode")try{let r=await F(u(z(e,t.env),"agents","iii.md"),"utf8");return t.stdout(`${U(C(r))}
`),0}catch(r){return t.stderr(`iii plugin: ${y(r)}
`),1}let n=null;try{if(i==="session-start"){let o=await(async()=>{let s=w(e,t.env);return n=new d(new g(s.url,t.fetchFn)),{profile:await n.getAgent(h)}})().catch(s=>({error:y(s)}));return t.stdout(`${L(o)}
`),0}let r=w(e,t.env);if(n=new d(new g(r.url,t.fetchFn)),i==="sync")await st(n,e,t);else if(i==="export")await ot(n,e,t);else if(i==="status"){let o=await n.status(),s=await n.listAgents(!0);t.stderr(`${JSON.stringify({mcp_server:r,...o,agents:s.length},null,2)}
`)}else return t.stderr(`unknown command: ${i} (expected sync, export, status, session-start or mode)
`),2;return 0}catch(r){return t.stderr(`iii plugin: ${y(r)}
`),1}finally{await n?.close()}}var at=await W(process.argv.slice(2),{env:process.env,fetchFn:fetch,stdout:e=>process.stdout.write(e),stderr:e=>process.stderr.write(e)});process.exit(at);
