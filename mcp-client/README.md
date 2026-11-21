# mcp-client

MCP client. Connects to the MCP servers you configure and registers their tools as iii functions (`mcp::<server>::<tool>`), with credentials read from the secrets worker.

## Install

```
iii call compose::add worker=mcp-client
```
