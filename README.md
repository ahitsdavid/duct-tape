# duct-tape

discord bot i wrote to stop alt-tabbing between 10 browser tabs to manage my homelab. DM it slash commands to check on things from my phone.

owner-only -- if you're not me, the bot ignores you.

## commands

- `/unraid` -- server status, disks, docker containers, VMs
- `/plex` -- library stats, recently added, who's streaming
- `/sonarr` `/radarr` -- manage tv shows and movies
- `/prowlarr` -- search indexers
- `/request` -- search for media and add it to sonarr/radarr in one go
- `/qbit` -- list/pause/resume torrents
- `/health` -- ping all services, see what's dead
- `/claude` -- talk to a claude/openai-compatible backend
- `/notes` -- read/write/search my obsidian vault from discord

you only need to configure the ones you actually use. leave a section out of `config.toml` and that plugin just doesn't load.

## running it

```bash
cp config.toml.example config.toml
cp .env.example .env
# fill both of those out, then:
docker compose up -d --build
```

## building

needs rust. there's a nix flake if you're into that.

```bash
nix develop
cargo build
cargo test --workspace
```

## how it works

each plugin implements a trait, registers its slash commands, handles its own interactions. core just does routing and auth. `plugins/` has the plugins, `crates/` has the core. not much else to say.

## thanks

- [serenity](https://github.com/serenity-rs/serenity) for making discord bots in rust not painful
- all the self-hosted projects this thing talks to

## license

MIT.
