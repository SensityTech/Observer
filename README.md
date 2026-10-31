# Observer

A server manager for FiveM and RedM, written in Rust.

Observer starts and supervises FXServer, installs its builds and serves a web panel for your staff,
from one executable that keeps its state in SQLite. It never shows anything to players: it raises
documented server events, and your own resources decide what to do with them.

> **Status: proof of concept.** Settings, the panel and the integration contract may still change
> between versions.

- **Supervision.** Start, stop and restart with a graceful stop. Crashes and hangs are detected and
  recovered from, a crash loop stops the retries, and FXServer's whole process tree stops with it.
- **Live console.** Search and filters, commands gated by permission, secrets masked, daily log files.
- **Scheduled restarts.** Fixed times or an interval in the time zone you choose, with warnings to your
  resources, skip, delay and one-off restarts.
- **FXServer builds.** The Cfx.re catalog, verified downloads, a switch at the next start or restart,
  probation, and a return to the last healthy build.
- **Statistics and diagnostics.** Players, tick times, CPU and memory of FXServer and the host,
  FXServer's threads and connections, crash dumps and hangs.
- **Accounts and audit.** An owner and sub-users with fine-grained permissions, and an audit log of every
  action.

## Install on Windows

Windows 10 or Windows Server 2016 and later, x64.

1. From the release, download `observer-<version>-windows-x86_64.zip` and `SHA256SUMS`, and check the
   archive: `Get-FileHash .\observer-<version>-windows-x86_64.zip` must print the hash written next to
   its name in `SHA256SUMS`.
2. Extract it into a folder of its own, such as `C:\Observer`. It holds `observer.exe` and
   `THIRD-PARTY-LICENCES.txt`. Keep the path short: FXServer fails on paths over 260 characters, and its
   deepest files sit 125 characters below the build folders Observer keeps under `data\artifacts\`.
3. Start `observer.exe` from that folder. Observer reads `observer.toml` from the folder it is started
   from, or the file `--config` names, and keeps `data\` next to it: from a terminal elsewhere, run
   `C:\Observer\observer.exe --config C:\Observer\observer.toml`.

The executable is not signed: on its first run, SmartScreen says "Windows protected your PC". Choose
**More info**, then **Run anyway**.

Observer then shows three things:

- **The console window**, with Observer's log. Ctrl+C in it stops FXServer gracefully, then Observer.
  Closing it stops both too, but Windows leaves only a few seconds, so FXServer quits at once.
- **The status window**: the server's state, players, build, next restart and the panel's address, with
  Start, Stop, Restart, **Open the panel** and **Open the data folder**. Until the owner account exists,
  it also holds the setup link. Closing the window leaves Observer running.
- **The tray icon**, whose menu reopens the window and has the same controls. **Quit Observer…** asks for
  confirmation, then stops FXServer and Observer.

The window and the tray sign nobody in: whoever uses the Windows session already holds the data folder,
and with it every account. The audit log shows their actions as **Host window**, and the events your
resources receive name them `local`.

`observer.exe run --no-window` starts Observer without the window and the tray, for a host where nobody
watches the desktop. The panel listens on every interface by default: allow Observer through the
firewall only on the networks your staff use, or see [HTTPS](#https).

## Install on Linux

x86_64 with glibc 2.35 or later (Ubuntu 22.04, Debian 12 and later).

```sh
sha256sum -c --ignore-missing SHA256SUMS      # where the archive and SHA256SUMS were downloaded
mkdir -p ~/observer
tar -xzf observer-<version>-linux-x86_64.tar.gz -C ~/observer
cd ~/observer
./observer
```

The archive holds `observer` and `THIRD-PARTY-LICENCES.txt`. Run Observer as the account that owns the
server folder, not as root: FXServer runs as the same account. As on Windows, `observer.toml` is read
from the folder Observer is started from, unless `--config` names it, and `data/` sits next to it.

`./observer` is short for `./observer run`. It runs in the foreground: Ctrl+C, SIGTERM and SIGHUP (a
closed SSH session) stop FXServer gracefully, then Observer. A server that must stay up runs under
[systemd](#systemd).

## First run

On its first start, Observer prints the link that creates the owner account:

```text
Create the owner account within 30 minutes, from this link:
http://localhost:40125/setup#<token>
```

- The link goes to the terminal only, never to the log files: whoever holds it can take the server. On
  Windows, the status window shows it too, with **Open the setup page** and **Copy the link**.
- It works once, within 30 minutes. While no account exists, `observer setup-link` prints a new one once
  Observer has started a first time, even while it runs. Under systemd nobody sees the terminal, so it is
  the only way there.
- When Observer listens on every interface, the link names `localhost`, which only a browser on the host
  itself reaches. From another machine, put the host's address in its place and keep the rest:
  `http://192.0.2.10:40125/setup#<token>`.
- The API answers to addresses and `localhost`. A name such as `panel.example.com` must be listed in
  `http.hosts`: until it is, the panel says Observer does not answer to this name.

Once the owner exists, the panel opens the server setup. Other accounts are added on the **Users**
page, each with a temporary password that works for 72 hours and is changed at the first sign-in.

From the host, even while Observer runs:

```sh
observer user list                    # accounts, roles and permissions
observer user reset-password <name>   # a temporary password, and the account's sessions end; the owner too
observer user unlock <name>           # lift the login locks on an account
```

## The server setup page

Observer runs an existing server in place. The setup opens by itself for the owner while nothing is
configured, and again from **Server → Run the setup**. It has four steps:

1. **Server folder.** The full path on the host of the folder that holds `resources` and the cfg, such
   as `C:\FXServer\server-data` or `/srv/fxserver/server-data`. Network paths are not supported.
   FXServer runs from this folder: Observer copies nothing and writes nothing into it.
2. **Config file.** The cfg FXServer executes, relative to the folder, such as `server.cfg`. Observer
   reads it and the cfgs it runs in turn, never changes them, and lists what it finds with file and line:
   blocking problems, warnings and notes, and what the cfg sets (game, endpoints, slots, OneSync,
   hostname, whether a licence key is set). Fix your files, then **Check again**; only a blocking problem
   stops the setup.
3. **Build.** Install the build Cfx.re recommends, or import a folder of FXServer you already have. On
   Windows, `FXServer.exe` must carry a valid Rockstar Games signature. **Skip for now** leaves it to the
   **Artifacts** page.
4. **Start.** OneSync, **Start FXServer with Observer**, **Send txAdmin's events too**, and a summary.
   **Save and start** (**Save and restart** when FXServer runs) saves the settings and starts FXServer,
   and the **First launch** checklist follows it: started, port open, licence accepted, Observer bridge
   connected, server ready. Without a build, **Save only** keeps the settings for later.

The **Server** page holds the same settings, and extra arguments that go on the command line as
`+set <name> <value>` after the cfg. It refuses secrets there, since anyone on the host can read a
command line: `sv_licenseKey`, passwords and connection strings belong in the cfg.

Builds go to `data/artifacts/<build>/`. The **Artifacts** page lists the catalog, installs and imports
builds, chooses the one FXServer runs, and cleans up old ones. A new build is on probation: until it has
stayed up 10 minutes once ready, a crash or a hang sends FXServer back to the last healthy build.

## systemd

`observer service print-unit` prints a unit that runs this binary with this configuration, as the
account that runs the command. Run it as the account meant to own the server: it writes nothing, and
prints on stderr the commands that install the unit, with the binary's and the configuration's full
paths, such as:

```sh
/home/fivem/observer/observer --config /home/fivem/observer/observer.toml service print-unit | sudo tee /etc/systemd/system/observer.service
sudo systemctl daemon-reload && sudo systemctl enable --now observer
/home/fivem/observer/observer --config /home/fivem/observer/observer.toml setup-link
```

The unit:

- is `Type=notify`: systemd counts Observer as started once the panel answers, whatever FXServer does;
- runs `<binary> --config <observer.toml> run` as that account, from the configuration's folder;
- restarts Observer on failure, after 5 seconds;
- on stop, sends SIGTERM to Observer alone, which stops FXServer gracefully, then the rest of the
  group (`KillMode=mixed`, `TimeoutStopSec=45`);
- makes `/usr`, `/boot`, `/efi` and `/etc` read-only (`ProtectSystem=full`), with a private `/tmp`:
  keep Observer's folder and the server's elsewhere, such as `/srv` or a home folder.

Printed as root, it warns that Observer and FXServer would run as root.

```sh
systemctl status observer      # Status: "FXServer online · 12 / 64 players"
journalctl -u observer -f      # the log, without colour codes
sudo systemctl stop observer   # FXServer stops gracefully first
```

Under systemd the setup link is never printed at start: while no account exists, the log says to run
`observer setup-link`.

## Configuration

Observer reads `observer.toml` from the folder it is started in, or the file `--config` names; the
option works with every command. A missing file means the defaults. An unknown key stops Observer with
an error rather than being ignored. Observer reads the file at start only: restart it after a change.
Everything else, such as the launch settings, the schedule and the accounts, is set in the panel and
kept in the database.

Every key, with its default:

```toml
# The database, its backups, the logs and the FXServer builds. A relative path is resolved against
# the folder of observer.toml.
data_dir = "data"

[http]
# The panel's address and port. 40120 and 40122 stay free for txAdmin.
listen = "0.0.0.0:40125"
# The names the panel is reached by, without a port, such as "panel.example.com". Addresses and
# localhost always work; the API refuses any other name, which a stranger's page could point at
# this host.
hosts = []
# The reverse proxies whose X-Forwarded-For and X-Forwarded-Proto are believed: addresses and CIDR
# blocks, IPv4 or IPv6, such as "127.0.0.1" or "172.18.0.0/16". An invalid entry stops Observer
# with an error that names it.
trusted_proxies = []

[log]
# A tracing filter, such as "debug" or "info,observer_web=debug".
level = "info"
```

Two environment variables apply to the log of `observer run` and `observer fxserver run`:

- `OBSERVER_LOG` replaces `log.level`, with a filter of the same form.
- `NO_COLOR`, set to anything but an empty value, turns off colour codes in the terminal. There are none
  when the log does not go to a terminal anyway, and never in the log files.

## Reference

### Data folder

```text
data/
├── observer.sqlite3       the state: accounts, sessions, audit log, settings, schedule, builds
├── metrics.sqlite3        the statistics, never backed up
├── observer.lock          held by the Observer that uses this folder
├── backups/               observer-<UTC time>-<kind>.sqlite3
├── logs/
│   ├── observer.<date>.log           Observer's log, one file a day, the last 14 kept
│   └── console/console-<date>.log    FXServer's console, 14 days and 2 GiB at most
├── artifacts/
│   ├── <build>/           a build of FXServer
│   ├── .staging/          downloads and extractions in progress
│   └── .trash/            deleted builds on their way out
├── crashes/               FXServer's crash dumps and hang snapshots: the last 10, 20 GiB at most
└── fxserver.pid           Linux: the running FXServer, which the next start ends if Observer died
```

While Observer runs, each database also has its `-wal` and `-shm` files. A console log file past 256 MiB
goes on in `console-<date>.<n>.log`.

The backups are complete snapshots of the state database:

| Kind | When | Kept |
|---|---|---|
| `daily` | at start when none is younger than a day, then once a day | 7 |
| `pre-migration` | before a new version changes the database's schema | 3 |
| `manual` | `observer db backup`, while Observer is stopped | 10 |

### Ports

| Port | What |
|---|---|
| 40125/TCP (`http.listen`) | The panel, its API and its websocket (`/api/ws`). |
| The cfg's endpoints, such as 30120 TCP and UDP | FXServer's own. Observer reads FXServer's HTTP endpoint (`/dynamic.json`, `/info.json`, `/perf/`, `/players.json`) on the cfg's first `endpoint_add_tcp`, through loopback when that one listens on every interface. |

### Outbound connections

Observer itself only talks to Cfx.re, over HTTPS on port 443, when someone lists or installs builds
from the panel, the setup or `observer artifacts`. It follows redirects only to the same hosts and
ignores proxy settings. FXServer's own traffic is FXServer's.

| Host | For |
|---|---|
| `docs.fivem.net` | the download page: recommended and latest builds |
| `changelogs-live.fivem.net` | recommended and latest builds, end of support and end of life |
| `runtime.fivem.net` | the build listings of each branch, and the archives |

Cfx.re's [Platform License Agreement](https://fivem.net/terms) covers the server builds you download.

### Command line

| Command | |
|---|---|
| `observer [run]` | Run Observer in the foreground. `--no-window` on Windows: no status window, no tray. |
| `observer setup-link` | A new link that creates the owner, while no account exists. |
| `observer user list` | Accounts, roles and permissions. |
| `observer user reset-password <name>` | A temporary password, and the account's sessions end. |
| `observer user unlock <name>` | Lift the login locks on an account, from every network. |
| `observer artifacts list` | This platform's builds with channel, dates and support. `--platform windows\|linux`, `--branch <name>` for a test branch, `--all` for old builds only the support policy names. |
| `observer artifacts install <build>` | Download, verify and install a build into `data/artifacts/`. `--branch <name>`, `--allow-end-of-life`. |
| `observer db check` | The database's integrity and schema version, while Observer is stopped. |
| `observer db backup` | A manual backup, while Observer is stopped. |
| `observer fxserver run --build <folder> --data <folder>` | FXServer under the supervisor in the foreground, without the panel or the database, while Observer is stopped. `--cfg`, `--onesync`, `--port`, `--set NAME=VALUE`. |
| `observer service print-unit` | Linux: the systemd unit, see [systemd](#systemd). |

The `user`, `setup-link` and `artifacts` commands work while Observer runs. `--version` names the commit
a release was built from.

## HTTPS

Observer serves plain HTTP and no TLS of its own. Passwords and console commands cross the network in
clear text unless the panel stays on the host, sits behind a reverse proxy that terminates HTTPS, or is
reached over a VPN.

- At start, when `listen` is not a loopback address and no proxy is trusted, Observer logs: `the panel
  is served over plain HTTP on 0.0.0.0:40125; put it behind an HTTPS proxy or reach it over a VPN`.
- Opened over plain HTTP at a name other than `localhost` or a name under it, such as
  `observer.localhost`, an address in `127.0.0.0/8` or `[::1]`, the panel shows a warning that cannot
  be dismissed: **Unencrypted connection.** It goes once the page is served over HTTPS.

Behind a proxy, set three keys:

```toml
[http]
listen = "127.0.0.1:40125"           # the proxy on the same host: no direct plain-HTTP access
hosts = ["panel.example.com"]        # the public name
trusted_proxies = ["127.0.0.1"]      # the proxy, as Observer sees it
```

- `trusted_proxies` holds the proxy's address as Observer sees it: `127.0.0.1` for a proxy on the same
  host. When Traefik runs in Docker and Observer on the host, it is the Docker network's block, such as
  `172.18.0.0/16` (`docker network inspect` shows it); Observer must then listen on an address the
  container reaches, with the port closed to the outside. Trust nothing wider.
- Trusting `127.0.0.1` trusts every process on the host, FXServer's resources included
  (`PerformHttpRequest`): any of them can claim any client address, which weakens the login throttling
  by address and the address in the audit log. A proxy in Docker, trusted by its network's block,
  avoids this.
- `X-Forwarded-For` and `X-Forwarded-Proto` are read from trusted peers only, and the client's address
  is the first entry from the right that is not a trusted proxy. The proxy must write those headers
  itself, not pass on the client's. `X-Forwarded-Host` and `Forwarded` are never read.
- The proxy must pass the browser's `Host` unchanged: Observer accepts a change, and the websocket's
  opening, only from the origin `<scheme>://<Host>`, exactly.
- Behind HTTPS, the session cookie gets `Secure`, and every response carries
  `Strict-Transport-Security: max-age=31536000`, without `includeSubDomains`. A proxy that adds its own
  sends the header twice.
- A browser that saw that header reaches the name over HTTPS only, on every port, for a year. Give the
  panel a name of its own, which no plain-HTTP service shares: txAdmin on port 40120 under the same name
  would no longer open in that browser.
- The setup link names the `listen` address. Behind the proxy, open the same path and fragment on the
  public name: `https://panel.example.com/setup#<token>`.

### Traefik

Traefik passes the original `Host` (`passHostHeader`, on by default: leave it on), sets
`X-Forwarded-For` and `X-Forwarded-Proto`, and upgrades the websocket by itself. It drops the
`X-Forwarded-*` headers a client sends unless the entry point trusts it: never set
`forwardedHeaders.insecure`. A router in the file provider's dynamic configuration:

```yaml
http:
  routers:
    observer:
      rule: Host(`panel.example.com`)
      entryPoints: [websecure]
      service: observer
      tls:
        certResolver: letsencrypt
  services:
    observer:
      loadBalancer:
        servers:
          - url: http://127.0.0.1:40125
```

### nginx

```nginx
# In the http block.
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    server_name panel.example.com;
    ssl_certificate     /etc/letsencrypt/live/panel.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/panel.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:40125;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
    }
}
```

When the public address carries a port, such as `https://panel.example.com:8443`, use
`proxy_set_header Host $http_host;`: `$host` drops the port, and the origin would no longer match.
Observer pings the websocket every 20 seconds, well within nginx's default 60-second
`proxy_read_timeout`. The panel opens `wss://` by itself when the page is HTTPS.

## Integrations

[`resource/CONTRACT.md`](resource/CONTRACT.md) is the integration contract: the server events Observer
raises before a restart or a stop (`observer:restartScheduled`, `observer:shuttingDown`, ...), what they
carry, in which order, and what your resources can rely on. The bridge that raises them, a server-side
resource named `observer`, is written into the build before every start of FXServer: do not ensure it
yourself. [`resource/observer_example`](resource/observer_example) listens to each event and marks where
a team plugs in its notifications and its saves.

Resources written for txAdmin keep working with **Send txAdmin's events too**, in the launch settings
and the setup's last step: from the next start of FXServer, the bridge also raises txAdmin's events,
such as `txAdmin:events:scheduledRestart` and `txAdmin:events:serverShuttingDown`, right after Observer's
own. The contract maps each one.

## Upgrading

1. Stop Observer: **Quit Observer…** in the tray, Ctrl+C in its console, or `sudo systemctl stop
   observer`. FXServer stops with it.
2. Replace `observer.exe` or `observer` with the new version's. Keep `observer.toml` and the data
   folder. Under systemd, keep the binary's path, or print and install the unit again.
3. Start it. A version whose database schema is newer first writes a `pre-migration` backup to
   `data/backups/`, then migrates; its log names the backup.

An older version refuses a database a newer one has migrated:
`the database was written by a newer Observer (schema 8, this build knows 7); use that version or
restore a backup`. To go back, restore the `pre-migration` backup the newer version wrote, as below,
then start the older version. It sets aside a statistics database it cannot read, as
`metrics.sqlite3.set-aside-<time>`, and starts a new one.

### Restoring a backup

1. Stop Observer, and make sure it has stopped: `observer db check` refuses to run while it does.
2. In the data folder, move `observer.sqlite3` aside, with `observer.sqlite3-wal` and
   `observer.sqlite3-shm` when they are there. A crash leaves them behind; they belong to the database
   they sit next to and must not meet the backup.
3. Copy the backup from `backups/` to `observer.sqlite3`.
4. Run `observer db check`. It prints `integrity: ok` and the backup's schema next to this build's. An
   older schema is migrated at the next start, after a `pre-migration` backup.
5. Start Observer.

Everything written since the backup is lost, and what was undone since comes back: a session ended
since is valid again until it expires, and a password changed since works again. The statistics, the
console logs and the builds are not part of the backup and stay as they are.

## Building from source

You need Rust through rustup (`rust-toolchain.toml` pins the version, which installs itself), Node.js 24
with npm, and a C compiler: the Visual Studio Build Tools on Windows, gcc or clang on Linux.

```sh
git config core.hooksPath .githooks   # once per clone: main only moves through pull requests
cargo xtask web                       # build the panel into web/dist
cargo run -p observer                 # Observer, with the panel on http://localhost:40125
cargo xtask check                     # everything CI checks
cargo xtask dist                      # the release archive and its checksum, in target/dist
```

`cargo xtask dist` builds for the host; on Linux,
`cargo xtask dist --target x86_64-unknown-linux-gnu --max-glibc 2.35` also refuses a binary that needs a
newer glibc. While working on the panel, `npm run dev` in `web/` serves it on http://localhost:41820
with hot reload and forwards `/api` to a running Observer (`OBSERVER_URL` names another one).
`cargo xtask bindings` regenerates the TypeScript types of the API from the Rust ones.
`cargo xtask e2e` runs the browser tests against a debug Observer built with a test catalog, from which
the flows install a build; a release never has that catalog, and `cargo xtask dist` refuses a binary
that does.

## Known limits

- **One operating system account for Observer and FXServer.** Any resource FXServer runs can read and
  change Observer's files, the database included, and so take the owner account. Run only resources you
  trust.
- **Signing out of Windows stops everything.** Observer runs in the Windows session it was started
  from: signing out, over RDP too, stops FXServer and Observer. Disconnecting from RDP, or closing the
  RDP window, leaves them running.
- **No Windows service.** Observer starts when someone starts it, in a signed-in session.
- **The status window's memory.** Once opened, the window keeps its graphics memory until Observer exits,
  even after it is closed. `--no-window` avoids it.
- **Colour codes in the classic console.** The classic console window (conhost), still the default on
  Windows 10 and Windows Server 2016 to 2022, prints the log's colour codes as text. Windows Terminal
  renders them, and `NO_COLOR=1` turns them off.
- **English only.** The panel, the command line and the logs are in English.

## Licence

No licence has been chosen yet. Until one is, all rights are reserved. The third-party software Observer
ships is listed with its licences in `THIRD-PARTY-LICENCES.txt` and on the panel's **About** page.
