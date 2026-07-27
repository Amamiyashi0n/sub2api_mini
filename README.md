# Sub2API Mini

Single-process Rust/Axum multi-user AI gateway with SQLite and an embedded console.

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
User authentication is local username/email and password only. The account console follows the original five-platform matrix:

- Anthropic: Claude Code OAuth/Setup Token, Claude Console API Key, AWS Bedrock, and Vertex Service Account.
- OpenAI: Codex OAuth and API Key.
- Gemini: Gemini CLI OAuth, AI Studio API Key, and Vertex Service Account.
- Antigravity: OAuth and Anthropic-compatible Upstream API Key.
- Grok: xAI OAuth and API Key.

Native gateway coverage is intentionally narrower than the original distributed backend. Anthropic direct/OAuth supports Messages and token counting; Bedrock and Vertex support non-streaming Messages; Antigravity Upstream supports Messages. OpenAI and Grok support Responses, Chat Completions, and Models. Gemini API Key supports Chat Completions and Models through Google's OpenAI-compatible endpoint. Gemini, Antigravity, and Grok OAuth tokens use provider-specific refresh endpoints; the two Google native OAuth inference protocols are stored and synchronized but are not yet gateway targets.

Third-party OAuth Client IDs and secrets are never built into the binary. Store them per account during token import, or set `SUB2API_MINI_GEMINI_OAUTH_CLIENT_ID`, `SUB2API_MINI_GEMINI_OAUTH_CLIENT_SECRET`, `SUB2API_MINI_ANTIGRAVITY_OAUTH_CLIENT_ID`, `SUB2API_MINI_ANTIGRAVITY_OAUTH_CLIENT_SECRET`, and `SUB2API_MINI_GROK_OAUTH_CLIENT_ID` in the runtime environment as needed.

Account proxies support HTTP, HTTPS, SOCKS5, and SOCKS5H URLs with encrypted credentials. A proxy can fail closed, fall back to a designated backup proxy, or explicitly fall back to a direct connection. The selected policy applies consistently to OAuth code exchange and refresh, model discovery, account tests, service-account token exchange, and gateway inference requests.
