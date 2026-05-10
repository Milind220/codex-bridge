# codex-bridge

Small Rust CLI that reads your local Codex CLI auth file (`~/.codex/auth.json`) and prints a usable access token for other local tools.

Goal: reuse your existing ChatGPT/Codex login without copying API keys around.

## Install

From source:

```bash
cargo install --path .
```

Binary commands:

- `codex-bridge print-token` -> prints access token to stdout
- `codex-bridge status` -> prints non-secret token health metadata
- `codex-bridge refresh` -> force refresh (requires refresh env vars, see below)

## How it works

1. Reads auth file from:
   - `CODEX_AUTH_FILE` (exact file path), else
   - `CODEX_HOME/auth.json`, else
   - `~/.codex/auth.json`
2. Parses `tokens.access_token` + `tokens.refresh_token`
3. Decodes JWT `exp` to check expiry
4. `print-token` refreshes automatically when token is near expiry

## Refresh behavior

For refresh calls, set:

- `CODEX_OAUTH_CLIENT_ID`
- optional `CODEX_OAUTH_TOKEN_URL` (default: `https://auth.openai.com/oauth/token`)

If refresh fails, run `codex login` and retry.

## Flue integration

In `.flue/app.ts`:

```ts
import { execFileSync } from 'node:child_process';
import { configureProvider } from '@flue/sdk/app';

const token = execFileSync('codex-bridge', ['print-token'], { encoding: 'utf8' }).trim();
configureProvider('openai-codex', { apiKey: token });
configureProvider('openai-codex-responses', { apiKey: token });
```

Then in your agent:

```ts
const agent = await init({ model: 'openai-codex/gpt-5-codex' });
```

## Security notes

- Never logs secrets.
- `print-token` writes token only to stdout.
- Keep your shell history clean when testing.
- Treat `~/.codex/auth.json` as sensitive.

## Dev

```bash
cargo test
cargo run -- status
cargo run -- print-token
```
