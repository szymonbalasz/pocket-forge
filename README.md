# Pocket Forge

A desktop asset studio for preparing D&D sessions. It generates, refines, organises and reuses campaign artwork (battlemaps, tokens, item art) through deliberate, controlled AI-assisted iteration.

**Status:** pre-alpha. The skeleton runs: import an image, open it as an asset project, pan and zoom, save and reopen.

## Develop

Needs Node 24 (corepack), Rust (`rust-toolchain.toml` pins it; rustup installs it) and, on Windows, WebView2.

```sh
corepack pnpm install
corepack pnpm env:init      # claims a dev port and a .pf-dev/ data + library folder for this worktree
corepack pnpm dev           # Vite + the Tauri window, on the fake provider
corepack pnpm check         # targeted checks for what you changed (--all for everything)
corepack pnpm e2e           # Playwright on the UI with a mocked core
corepack pnpm tauri:smoke   # build the app and check the window loads
corepack pnpm fixtures      # generate fixture images into fixtures/generated/
```

## Install a local build (Windows)

```sh
corepack pnpm app:install          # release build → %LOCALAPPDATA%\Programs\Pocket Forge, plus Desktop and Start menu shortcuts
corepack pnpm app:install --hook   # also rebuild and reinstall in the background after every merge or pull on main
```

The installed app is a release build: it uses Scenario and the real prompt chat, and keeps its data apart from dev runs. A running app is never interrupted; the new build starts on the next launch. A failed build leaves the installed app as it was (see `install.log` in the install folder). `--unhook` removes the hook.

Build conventions: [AGENTS.md §9](AGENTS.md#9-build-conventions).

- Vision: [docs/vision.md](docs/vision.md)
- How the project is built: [docs/process.md](docs/process.md)
- Contributor and agent rules: [AGENTS.md](AGENTS.md)

Planned platforms: Windows first, then macOS.

Licence: [MIT](LICENSE).
