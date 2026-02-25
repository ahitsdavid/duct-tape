# DiscordAssist

Rust Discord bot with trait-based plugin system.

## Development

Enter dev shell: `nix develop`
Build: `cargo build`
Test: `cargo test --workspace`
Lint: `cargo clippy --workspace -- -D warnings`
Watch: `cargo watch -x 'test --workspace'`

## Project Structure

- `crates/core/` -- Bot startup, Discord gateway, command routing, owner-only auth
- `crates/plugin-api/` -- Plugin trait and shared types
- `plugins/unraid/` -- Unraid server management via GraphQL API
- `plugins/claude/` -- Claude AI assistant with conversation tracking
- `plugins/sonarr/` -- Sonarr TV show management
- `plugins/radarr/` -- Radarr movie management
- `plugins/prowlarr/` -- Prowlarr indexer management
- `plugins/arr-common/` -- Shared *arr REST client

## Adding a New Plugin

1. Create `plugins/<name>/` with `Cargo.toml` and `src/lib.rs`
2. Implement `Plugin` trait from `discord-assist-plugin-api`
3. Add to workspace members in root `Cargo.toml`
4. Add dependency and registration in `crates/core/`
5. Add config section in `crates/core/src/config.rs`
6. `docker compose build && docker compose up -d`

## Deployment

```bash
cp config.toml.example config.toml  # Edit with your values
cp .env.example .env                # Add secrets here
docker compose up -d --build
docker compose logs -f
```

## Secrets

API keys and the Discord token can be set via environment variables instead of `config.toml`.
Env vars override TOML values when set and non-empty; they only apply to plugin sections
that already exist in the TOML file. See `.env.example` for the full list.

| Env Var | Overrides |
|---------|-----------|
| `DISCORD_TOKEN` | `discord.token` |
| `UNRAID_API_KEY` | `unraid.api_key` |
| `CLAUDE_API_KEY` | `claude.api_key` |
| `SONARR_API_KEY` | `sonarr.api_key` |
| `RADARR_API_KEY` | `radarr.api_key` |
| `PROWLARR_API_KEY` | `prowlarr.api_key` |
