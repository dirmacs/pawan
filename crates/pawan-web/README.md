# pawan-web

HTTP API server for Pawan — Axum + SSE streaming for real-time agent interactions.

## Overview

`pawan-web` provides a RESTful HTTP API and Server-Sent Events (SSE) streaming interface for the Pawan coding agent. It enables web applications and other services to interact with Pawan's AI-powered coding capabilities through standard HTTP protocols.

## Features

- **RESTful API** — Standard HTTP endpoints for agent interactions
- **SSE Streaming** — Real-time streaming of agent responses and tool calls
- **Session Management** — Create, list, retrieve, and delete conversation sessions
- **Multi-Agent Support** — Manage multiple agent instances concurrently
- **Health Monitoring** — Built-in health check and status endpoints
- **Model Discovery** — List available models and their configurations
- **CORS Support** — Cross-origin resource sharing for web applications
- **Aegis Integration** — Peer discovery via Aegis network configuration

## Installation

Build from source:

```bash
cd /opt/pawan
cargo build --release --bin pawan-web
```

The binary will be available at `target/release/pawan-web`.

## Usage

### Starting the Server

```bash
# Start with default configuration
pawan-web

# Start with custom workspace
pawan-web --workspace /path/to/project

# Start with custom config
pawan-web --config /path/to/pawan.toml
```

The server starts on port 3300 by default.

### Configuration

The server reads configuration from `pawan.toml` in the workspace directory:

```toml
provider = "nvidia"
model = "qwen/qwen3.5-122b-a10b"
temperature = 0.6
max_tokens = 4096
```

## API Endpoints

### Health Check

```http
GET /health
```

Response:
```json
{
  "status": "ok",
  "version": "0.3.2",
  "uptime_secs": 1234,
  "agent_id": "pawan@hostname"
}
```

### List Agents

```http
GET /agents
```

Response:
```json
{
  "self": "pawan@hostname",
  "peers": [
    {
      "name": "agent1",
      "agent_id": "pawan@agent1",
      "ip": "192.168.1.100",
      "groups": ["production"]
    }
  ]
}
```

### List Models

```http
GET /models
```

Response:
```json
{
  "models": [
    {
      "name": "qwen/qwen3.5-122b-a10b",
      "provider": "Nvidia",
      "is_default": true
    },
    {
      "name": "minimaxai/minimax-m2.5",
      "provider": "Nvidia",
      "is_default": false
    }
  ]
}
```

### Chat (Non-Streaming)

```http
POST /chat
Content-Type: application/json

{
  "session_id": "optional-session-id",
  "message": "Fix the failing test in src/lib.rs",
  "model": "optional-model-override"
}
```

Response:
```json
{
  "session_id": "abc-123-def-456",
  "content": "I'll help you fix the failing test...",
  "iterations": 3,
  "tool_calls": 5
}
```

### Chat (Streaming)

```http
POST /chat/stream
Content-Type: application/json

{
  "session_id": "optional-session-id",
  "message": "Explain how the agent loop works",
  "model": "optional-model-override"
}
```

Response: Server-Sent Events stream

```
event: token
data: {"content": "The"}

event: token
data: {"content": " agent"}

event: tool_start
data: {"name": "read"}

event: tool_complete
data: {"name": "read", "success": true, "duration_ms": 42, "result_preview": "..."}

event: done
data: {"session_id": "abc-123", "content": "The agent loop works by...", "iterations": 2, "tool_calls": 3}
```

### Session Management

#### List Sessions

```http
GET /sessions
```

Response:
```json
[
  {
    "id": "abc-123-def-456",
    "model": "qwen/qwen3.5-122b-a10b",
    "created_at": "2026-04-14T12:00:00Z",
    "updated_at": "2026-04-14T12:05:00Z",
    "message_count": 10
  }
]
```

#### Get Session

```http
GET /sessions/{id}
```

Response:
```json
{
  "id": "abc-123-def-456",
  "model": "qwen/qwen3.5-122b-a10b",
  "created_at": "2026-04-14T12:00:00Z",
  "updated_at": "2026-04-14T12:05:00Z",
  "messages": [
    {
      "role": "user",
      "content": "Fix the test",
      "timestamp": "2026-04-14T12:01:00Z"
    },
    {
      "role": "assistant",
      "content": "I'll fix it...",
      "timestamp": "2026-04-14T12:02:00Z",
      "tool_calls": [...]
    }
  ]
}
```

#### Create Session

```http
POST /sessions
```

Response:
```json
{
  "session_id": "new-uuid-generated"
}
```

#### Delete Session

```http
DELETE /sessions/{id}
```

Response: `204 No Content`

## SSE Event Types

### token
Emitted for each token in the agent's response.

```json
{
  "content": "The"
}
```

### tool_start
Emitted when a tool execution begins.

```json
{
  "name": "read"
}
```

### tool_complete
Emitted when a tool execution completes.

```json
{
  "name": "read",
  "success": true,
  "duration_ms": 42,
  "result_preview": "fn main() {..."
}
```

### done
Emitted when the agent completes the request.

```json
{
  "session_id": "abc-123",
  "content": "Complete response...",
  "iterations": 3,
  "tool_calls": 5
}
```

### error
Emitted when an error occurs.

```json
{
  "message": "Failed to read file: permission denied"
}
```

## Client Examples

### cURL

```bash
# Health check
curl http://localhost:3300/health

# List models
curl http://localhost:3300/models

# Chat (non-streaming)
curl -X POST http://localhost:3300/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, Pawan!"}'

# Chat (streaming)
curl -X POST http://localhost:3300/chat/stream \
  -H "Content-Type: application/json" \
  -d '{"message": "Explain Rust"}'

# List sessions
curl http://localhost:3300/sessions
```

### JavaScript (Fetch API)

```javascript
// Non-streaming chat
const response = await fetch('http://localhost:3300/chat', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    message: 'Fix the failing test',
    session_id: 'my-session'
  })
});
const data = await response.json();
console.log(data.content);

// Streaming chat
const eventSource = new EventSource('http://localhost:3300/chat/stream', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ message: 'Explain Rust' })
});

eventSource.addEventListener('token', (e) => {
  const data = JSON.parse(e.data);
  console.log('Token:', data.content);
});

eventSource.addEventListener('done', (e) => {
  const data = JSON.parse(e.data);
  console.log('Complete:', data.content);
  eventSource.close();
});
```

### Python

```python
import requests
import sseclient

# Non-streaming
response = requests.post('http://localhost:3300/chat', json={
    'message': 'Hello, Pawan!'
})
print(response.json()['content'])

# Streaming
response = requests.post('http://localhost:3300/chat/stream', json={
    'message': 'Explain Rust'
}, stream=True)

client = sseclient.SSEClient(response)
for event in client.events():
    if event.event == 'token':
        print(event.data, end='', flush=True)
    elif event.event == 'done':
        print('\nComplete!')
        break
```

## Architecture

### Application State

The server maintains shared state across all HTTP handlers:

```rust
pub struct AppState {
    agents: Arc<RwLock<HashMap<String, PawanAgent>>>,
    config: Arc<PawanConfig>,
    workspace: PathBuf,
    agent_id: String,
    start_time: std::time::Instant,
}
```

### Session Management

- Sessions are stored in `~/.pawan/sessions/`
- Each session has a unique UUID
- Sessions persist across server restarts
- Automatic session resumption via `session_id`

### Agent Lifecycle

1. **Creation**: Agent created on first request with new `session_id`
2. **Execution**: Agent processes message with tool calling
3. **Callbacks**: Token, tool start, and tool completion events streamed
4. **Persistence**: Session saved automatically after completion
5. **Archival**: Optional archival to Eruka memory system

## CORS Configuration

The server includes CORS support for web applications:

```rust
let cors = CorsLayer::new()
    .allow_origin(Any)
    .allow_methods([Method::GET, Method::POST, Method::DELETE])
    .allow_headers(Any);
```

## Aegis Integration

The server can discover peer agents via Aegis network configuration:

```toml
# ~/.config/aegis/aegis-net.toml
[peers.agent1]
ip = "192.168.1.100"
groups = ["production"]

[peers.agent2]
ip = "192.168.1.101"
groups = ["development"]
```

Access via `/agents` endpoint.

## Error Handling

The server returns appropriate HTTP status codes:

- `200 OK` — Successful request
- `204 No Content` — Successful deletion
- `400 Bad Request` — Invalid request body
- `404 Not Found` — Session not found
- `500 Internal Server Error` — Agent execution error

Error responses include descriptive messages:

```json
{
  "error": "Failed to execute agent: permission denied"
}
```

## Development

### Running in Development

```bash
cd /opt/pawan
cargo run --bin pawan-web

# With custom workspace
cargo run --bin pawan-web -- --workspace /path/to/project
```

### Testing

```bash
# Run web server tests
cargo test --bin pawan-web

# Run with logging
RUST_LOG=debug cargo run --bin pawan-web
```

## Performance Considerations

- **Concurrent Sessions**: Multiple sessions handled via `RwLock<HashMap>`
- **Streaming**: SSE provides low-latency real-time updates
- **Memory**: Sessions persisted to disk, not kept in memory
- **Scalability**: Stateless design allows horizontal scaling

## Security

- **CORS**: Configurable for production deployments
- **Input Validation**: All requests validated before processing
- **File Access**: Agent respects workspace boundaries
- **No Authentication**: Currently designed for trusted environments

## License

MIT

## See Also

- [Pawan](https://github.com/dirmacs/pawan) — Main CLI coding agent
- [pawan-core](https://crates.io/crates/pawan-core) — Core library
- [Axum](https://github.com/tokio-rs/axum) — Web framework
- [SSE](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events) — Server-Sent Events
