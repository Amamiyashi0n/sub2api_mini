# Sub2API Mini

Single-process Rust/Axum multi-user gateway for Anthropic Claude and OpenAI with SQLite and an embedded console.

## Endpoints

- Admin UI: `http://localhost:8080`
- Admin API: `http://localhost:8080/api/admin`
- Authentication API: `http://localhost:8080/api/auth`
- User self-service API: `http://localhost:8080/api/user`
- Gateway: `http://localhost:8080/v1`
- Claude Messages: `POST http://localhost:8080/v1/messages`
- Claude token counting: `POST http://localhost:8080/v1/messages/count_tokens`
- OpenAI Responses: `POST http://localhost:8080/v1/responses`
- OpenAI Chat Completions: `POST http://localhost:8080/v1/chat/completions`
- Batch images: `http://localhost:8080/v1/images/batches`
- Health: `http://localhost:8080/health`
- OAuth callback: `http://localhost:1455/auth/callback`

## Development

```sh
cargo test
cargo run
```

Runtime configuration is loaded directly from `/data/sub2api_mini/.env`. The UI is embedded in the Rust binary, so UI changes require a rebuild. Credentials are encrypted with `SUB2API_MINI_MASTER_KEY`; changing that key makes existing upstream credentials unreadable.

The administrator creates and disables users. Each user has an isolated session, API keys, dashboard, and usage history. Upstream accounts remain administrator-managed and shared by the gateway scheduler.
User authentication is local username/email and password only. Administrator-managed upstream accounts support Claude Code OAuth/Setup Token, Anthropic API keys, OpenAI/Codex OAuth, and OpenAI API keys.
