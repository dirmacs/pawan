# pawan-mcp

MCP (Model Context Protocol) client integration for Pawan. Connects to MCP servers and bridges their tools into Pawan's tool system.

## Features

- **rmcp 0.12** — Rust MCP client implementation
- **Tool bridging** — MCP server tools appear as native Pawan tools
- **Multi-server** — Connect to multiple MCP servers simultaneously
- **Stdio transport** — Launch MCP servers as child processes

## Usage

```rust
use pawan_mcp::McpClient;

let client = McpClient::connect("npx", &["-y", "@modelcontextprotocol/server-filesystem"]).await?;
let tools = client.list_tools().await?;
```

## License

MIT
