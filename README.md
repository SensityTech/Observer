# Observer

A server manager for FiveM and RedM, written in Rust.

Observer starts and supervises FXServer, keeps it healthy, manages its builds and gives your staff a
secure web panel to run it. It is built for large servers run by developers: instead of shipping in-game
features, it exposes documented, versioned integration points that your own resources build on.

> **Status: early development.** A proof of concept is being built. Nothing here is ready for
> production yet, and interfaces may change without notice.

## Scope of the proof of concept

- **Supervision.** Starts, stops and restarts FXServer with a clean shutdown sequence. Detects crashes
  and hangs from several signals, recovers with backoff and stops on a crash loop. Owns the whole process
  tree, so nothing is left running behind it.
- **Live console.** Real-time output with search and filters, commands gated by permissions, secrets
  masked, timestamped and rotated log files.
- **Scheduled restarts.** Fixed times or any interval, time zone and daylight saving aware, with skip,
  postpone and one-off restarts.
- **Artifacts.** Recommended, latest and branch builds of FXServer. Verified downloads, safe extraction,
  switch on the next restart, probation after a switch and automatic rollback to the last good build.
- **Metrics.** Players, tick times, per-thread CPU, memory trends, process and host usage, kept as
  history with downsampling. Crash dumps and hangs are analysed, not just counted.
- **Accounts and audit.** Username and password sign-in, an owner and sub-users with fine-grained
  permissions, and an append-only audit log of every action.
- **One executable.** Embedded web panel and SQLite storage. A small status window on Windows; a CLI and
  a systemd unit on Linux.

## Principles

- **Integration over features.** Observer never shows anything to players. It emits server events with
  structured payloads (`observer:restartScheduled`, `observer:shuttingDown`, ...) and your resources
  decide what to do with them.
- **Hands off the game server.** Nothing is injected into FXServer. Everything Observer knows comes from
  the operating system, FXServer's own endpoints and a minimal server-side bridge.
- **FXServer never runs with more rights than it needs.** An optional elevated mode on Windows unlocks
  deeper diagnostics for Observer only.
- **Self-contained.** No account, telemetry or service is required beyond the Cfx.re artifact servers.

## Platforms

Windows x64 is the primary target. Linux x86_64 is supported.

## Development

You need Rust through rustup (the toolchain pinned in `rust-toolchain.toml` installs itself), Node.js
24 and npm.

```bash
git config core.hooksPath .githooks   # once per clone: main only moves through pull requests
cargo xtask web                       # build the panel into web/dist
cargo run -p observer                 # run Observer and serve the panel on http://localhost:40125
cargo xtask check                     # everything CI checks
```

While working on the panel, `npm run dev` in `web/` serves it on http://localhost:41820 with hot reload
and forwards `/api` to a running Observer (set `OBSERVER_URL` to reach another one).

`cargo xtask bindings` regenerates the TypeScript types of the API from the Rust ones, and the
permission constants the panel gates on from the Rust catalog; CI fails when the committed files are
out of date. `cargo xtask dist` builds a release binary with the panel inside.

### Configuration

Observer reads `observer.toml` from the working directory, or the file given with `--config`. Every key
is optional; relative paths are resolved against the file's directory.

```toml
data_dir = "data"            # database, backups and logs

[http]
listen = "0.0.0.0:40125"

[log]
level = "info"               # a tracing filter; the OBSERVER_LOG variable overrides it
```

`observer db check` and `observer db backup` inspect and back up the database while Observer is
stopped.

### First run and accounts

On its first start, Observer prints a link on the terminal that creates the owner account. The link
works for 30 minutes and never appears in the logs. Under systemd, or once it has expired, run
`observer setup-link` for a new one.

These commands work while Observer runs:

```bash
observer user list                    # accounts, roles and permissions
observer user reset-password <name>   # a temporary password; works for the owner too
observer user unlock <name>           # lift the login locks on an account
observer setup-link                   # a new owner-creation link, while no account exists
```

## License

No license has been chosen yet. Until one is, all rights are reserved.
