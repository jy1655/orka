# Security Policy

## Reporting a Vulnerability

Do not open public issues with exploit details, bot tokens, chat IDs, provider account data, or full logs.

Use GitHub private vulnerability reporting when available on this repository. If private reporting is unavailable, contact the repository owner out of band and include only the minimum reproduction detail needed to confirm impact.

## Live Operation Baseline

- Rotate Discord, Telegram, and provider credentials after any suspected exposure.
- Keep `OPEN_ACCESS=false` for live use.
- Keep `PUBLIC_CHAT=false` unless every member of every joined channel is trusted to spend provider tokens.
- Prefer `CHANNEL_ALLOWLIST=discord:<channel_id>,telegram:<chat_id>` for approved public channels.
- Keep `RATE_LIMIT_MAX_REQUESTS` non-zero in `RUNTIME_ENGINE=cli` deployments.
- Keep `STORE_FULL_PAYLOADS=false` unless short-lived local debugging requires full payload storage.
- Keep `HEALTH_BIND` on loopback unless protected by `HEALTH_BEARER_TOKEN` and an authenticated reverse proxy or network allowlist.
- Do not run the Windows service as `LocalSystem` for live use. Use the default virtual service account or a dedicated low-privilege user.
- Avoid importing `.env` into the Windows service registry. Prefer service/user environment variables or a credential loader.
