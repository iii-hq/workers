# Workers Being Migrated

End-state catalog of workers being migrated from `motia/harness/workers` into
this repository as standalone, publishable workers.

## Auth, Policy & Budget

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `auth-credentials` | `iii-auth-credentials` | Centralize provider secret storage so agents and providers never see raw API keys. | Provider credential vault under `auth::*` — stores and retrieves API keys and OAuth tokens. |
| `auth-rbac` | `iii-auth-rbac` | Enforce who can do what in a workspace by issuing API keys and assigning roles. | HMAC API keys and workspace roles (owner / admin / member / viewer) under `auth::rbac::*`. |
| `llm-budget` | `iii-llm-budget` | Prevent runaway LLM spend by capping cost per workspace and per agent with alerts before limits hit. | Workspace and agent LLM spend caps with alerts, forecasts, and period rollover under `budget::*`. |
| `guardrails` | `iii-guardrails` | Block prompts and responses that contain secrets, PII, or jailbreak patterns before they reach a provider or user. | Local heuristics for PII, leaked API keys, jailbreak keywords, and toxicity under `guardrails::*`. |
| `policy-denylist` | `iii-policy-denylist` | Reject tool calls and prompts matching configured denylist rules. | Hook subscriber enforcing denylist policy, split from `policy-subscribers`. |
| `audit-log` | `iii-audit-log` | Produce a tamper-evident record of every agent action for compliance and incident review. | Idempotent audit-log subscriber, split from `policy-subscribers`. |
| `dlp-scrubber` | `iii-dlp-scrubber` | Redact sensitive data (emails, tokens, PII) from prompts and outputs before they leave the workspace. | DLP scrubbing hook subscriber, split from `policy-subscribers`. |

## Session & Context

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `session-tree` | `iii-session-tree` | Persist agent conversation state as a structured tree so turns, branches, and child sessions can be replayed and inspected. | Session storage as a parent-id tree of typed entries under `session::*`. |
| `session-corpus` | `iii-session-corpus` | Turn finished sessions into clean datasets for evaluation, fine-tuning, and analytics. | Dataset publishing pipeline over completed sessions. |
| `context-compaction` | `iii-context-compaction` | Keep long sessions within model context windows by summarizing older history when thresholds are crossed. | Subscriber that compacts session context once thresholds are reached. |
| `models-catalog` | `iii-models-catalog` | Give the harness a single source of truth for model capabilities (context size, tools, vision, pricing) so routing and budget decisions are accurate. | Model capabilities knowledge base under `models::*` (list / get / supports / register). |
| `document-extract` | `iii-document-extract` | Let agents read PDFs and Word docs as plain text without each provider needing its own file ingestion. | PDF and Word text extraction under `document::extract` for agent context ingestion. |

## Runtime Core

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `turn-orchestrator` | `iii-turn-orchestrator` | Run each agent turn as a durable workflow that survives restarts, retries, and crashes mid-tool-call. | Durable `run::start` state machine driving each agent turn. |
| `provider-router` | `iii-provider-router` | Wire primitives, shells, and providers together so a deployment can register everything an agent needs in one call. | Fans in primitives and shells; either standalone or rolled into `harness`. |

## Shell Workers

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `shell-filesystem` | `iii-shell-filesystem` | Give agents read/write access to a workspace filesystem inside a sandbox boundary, never the host. | Sandboxed filesystem operations exposing `sandbox::fs::*`. |
| `shell-bash` | `iii-shell-bash` | Let agents run shell commands inside a sandbox without ever falling back to the host shell. | Sandboxed shell execution exposing `sandbox::exec` (no host fallback). |
| `shell-subagent` | `iii-shell-subagent` | Let an agent delegate work by spawning a child agent session with its own scope and budget. | Spawns child agent sessions via `run::start`. |

## Provider Adapters

Each adapter connects to `III_URL` and registers its provider with the
harness, exposing a unified completions surface so agents can swap models
without code changes.

### Native APIs

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `provider-anthropic` | `iii-provider-anthropic` | Talk to Anthropic with first-class support for tool use, thinking, and prompt caching. | Native Anthropic Messages API. |
| `provider-openai` | `iii-provider-openai` | Talk to OpenAI's classic Chat Completions endpoint for broad model coverage. | OpenAI Chat Completions. |
| `provider-openai-responses` | `iii-provider-openai-responses` | Use OpenAI's newer Responses API for stateful, tool-rich agent loops. | OpenAI Responses API. |
| `provider-google` | `iii-provider-google` | Talk to Google Gemini directly via the public AI Studio API. | Google Gemini API. |
| `provider-google-vertex` | `iii-provider-google-vertex` | Run Gemini in GCP with Vertex auth, regions, and enterprise controls. | Vertex AI Gemini. |
| `provider-azure-openai` | `iii-provider-azure-openai` | Run OpenAI models inside Azure with enterprise auth and data residency. | Azure OpenAI Responses shape. |

### OpenAI-Compatible

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `provider-openrouter` | `iii-provider-openrouter` | Reach hundreds of models behind one API for routing, fallback, and price comparison. | OpenRouter routing layer. |
| `provider-groq` | `iii-provider-groq` | Hit Groq's hardware for ultra-low-latency inference. | Groq inference. |
| `provider-cerebras` | `iii-provider-cerebras` | Hit Cerebras's wafer-scale inference for very high throughput. | Cerebras inference. |
| `provider-xai` | `iii-provider-xai` | Use xAI Grok models with their OpenAI-compatible endpoint. | xAI Grok models. |
| `provider-deepseek` | `iii-provider-deepseek` | Use DeepSeek's reasoning and coding models cheaply. | DeepSeek models. |
| `provider-mistral` | `iii-provider-mistral` | Use Mistral models hosted on La Plateforme. | Mistral La Plateforme. |
| `provider-fireworks` | `iii-provider-fireworks` | Run open-weights and fine-tuned models on Fireworks. | Fireworks AI. |
| `provider-kimi-coding` | `iii-provider-kimi-coding` | Use Moonshot Kimi's coding-optimized endpoint. | Moonshot Kimi coding endpoint. |
| `provider-minimax` | `iii-provider-minimax` | Use MiniMax models for multilingual and long-context workloads. | MiniMax models. |
| `provider-zai` | `iii-provider-zai` | Use Z.ai (GLM) models. | Z.ai models. |
| `provider-huggingface` | `iii-provider-huggingface` | Hit any model hosted on Hugging Face's Inference API. | Hugging Face Inference API. |
| `provider-vercel-ai-gateway` | `iii-provider-vercel-ai-gateway` | Route through Vercel's AI Gateway for caching, rate limits, and observability. | Vercel AI Gateway. |
| `provider-opencode-zen` | `iii-provider-opencode-zen` | Reach the opencode Zen model endpoint. | opencode Zen endpoint. |
| `provider-opencode-go` | `iii-provider-opencode-go` | Reach the opencode Go model endpoint. | opencode Go endpoint. |

### Local & Specialty

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `provider-cli` | `iii-provider-cli` | Use locally installed coding agents (claude-code, codex, gemini) as if they were a remote provider. | Drives local CLI tools (claude-code, codex, gemini) via `shell::bash`. |

## OAuth Workers

| Worker | Binary | Purpose | Description |
|---|---|---|---|
| `oauth-anthropic` | `iii-oauth-anthropic` | Sign in with Anthropic via PKCE so users authorize without pasting API keys. | Anthropic PKCE localhost OAuth flow. |
| `oauth-openai-codex` | `iii-oauth-openai-codex` | Sign in with the OpenAI Codex account to use it as a provider. | OpenAI Codex PKCE localhost OAuth flow. |
| `oauth-github-copilot` | `iii-oauth-github-copilot` | Authorize GitHub Copilot via device code so headless and remote installs work. | GitHub Copilot device-code flow. |
| `oauth-google-gemini-cli` | `iii-oauth-google-gemini-cli` | Sign in with the Google Gemini CLI account via PKCE. | Google Gemini CLI PKCE flow. |
| `oauth-google-antigravity` | `iii-oauth-google-antigravity` | Sign in with Google Antigravity via PKCE. | Google Antigravity PKCE flow. |
