# Observer

A server manager for FiveM and RedM, written in Rust.

Observer starts and supervises FXServer, installs its builds and serves a web panel for your staff,
from one executable that keeps its state in SQLite. It never shows anything to players: it raises
documented server events, and your own resources decide what to do with them.

> **Status: proof of concept.** Settings, the panel and the integration contract may still change
> between versions.

- **Supervision.** Start, stop and restart with a graceful stop. Crashes and hangs are detected and
  recovered from, a crash loop stops the retries (5 failures within 10 minutes, or 5 starts in a row
  that never got ready), and FXServer's whole process tree stops with it. A server stuck for 6 minutes
  is restarted; the delay can be raised, or hang detection turned off.
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
   FXServer runs from this folder. Observer copies nothing into it, and writes there only what people
   upload over [SFTP](#file-access-over-sftp) and the fixes of the [server tools](#server-tools) the
   owner runs or turns on: the **Tools** page lists each fix with its backup.
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

The **Server** page holds the same settings, [hang detection](#hang-detection), and extra arguments
that go on the command line as `+set <name> <value>` after the cfg. It refuses secrets there, since
anyone on the host can read a command line: `sv_licenseKey`, passwords and connection strings belong in
the cfg.

Builds go to `data/artifacts/<build>/`. The **Artifacts** page lists the catalog, installs and imports
builds, chooses the one FXServer runs, and cleans up old ones. A new build is on probation: until it has
stayed up 10 minutes once ready, a crash or a hang sends FXServer back to the last healthy build. A
server whose heartbeat or HTTP is late when those minutes end passes only once it answers again, so a
hang that began within them fails the probation however long the [hang delay](#hang-detection). With
hang detection off, only a crash fails it, and a silent server still waits to answer before it passes.

### Hang detection

Observer watches two signals while FXServer runs: the heartbeat of its bridge resource, which only
comes while the main loop turns, and FXServer's HTTP endpoints. Once the server is up, one of them
silent for 10 seconds marks it degraded. Silent for the delay under **Server → Hang detection**, 6
minutes by default and counted from the start until the first answer, the server is hung: Observer
writes a dump of it (a report of its threads on Linux), kills its process tree and starts it again.

A big server's first start can keep its main loop busy for minutes while resources build their caches of
clothing and vehicles, which reads as a hang. Raise the delay, from 1 to 60 minutes, or turn hang detection
off: FXServer is then left running however long it stays silent, until someone restarts it. Either change
applies at once, to a server already starting too, and the delay then counts from the change: turning
detection back on never kills a server that was silent before. A crash is another matter: whenever
FXServer's process exits on its own, Observer restarts it, whatever these settings say, until a crash loop
stops the retries.

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

# File access over SFTP stays off until this section exists (see "File access over SFTP").
# [sftp]
# listen = "0.0.0.0:40126"

# Optional features the host keeps off, which the panel cannot turn on (see "Turning features off").
# [features]
# sftp = "off"
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
├── changes.sqlite3        what each start of FXServer ran from, never backed up
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
├── tools/backups/         the originals of the files the server tools changed, each with a .json
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
| 40126/TCP (`sftp.listen`), when set | File access over SFTP. Off unless `[sftp]` is in `observer.toml` and SFTP is not [turned off](#turning-features-off). |
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
| `observer fxserver run --build <folder> --data <folder>` | FXServer under the supervisor in the foreground, without the panel or the database, while Observer is stopped. `--cfg`, `--onesync`, `--port`, `--set NAME=VALUE`, `--hang-timeout <minutes>` from 1 to 60 (6 by default) or `--no-hang-detection`, see [Hang detection](#hang-detection). |
| `observer service print-unit` | Linux: the systemd unit, see [systemd](#systemd). |
| `observer tools run duplicate-archetypes` | Find duplicate archetypes and print the report. `--fix` while Observer is stopped, `--json`, `--data <folder>`, `--cfg <file>` or `--all-resources`. Exits with 2 while duplicates are left, 1 on an error. See [Server tools](#server-tools). |
| `observer tools undo duplicate-archetypes` | Put back what fixes took out, while Observer is stopped: `--run <id>` or `--all`, and `--data <folder>`. |
| `observer tools config duplicate-archetypes` | Print the tool's settings, or change them: `--at-start`, `--differences`, `--escrowed`, `--exclude <name>`, `--include <name>`. |

The `user`, `setup-link` and `artifacts` commands, `tools run` without `--fix` and `tools config` work
while Observer runs. `--version` names the commit a release was built from.

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

## File access over SFTP

Observer can let people and CI pipelines read and change chosen parts of the server folder over SFTP,
the file transfer of SSH. It is off until the owner adds this to `observer.toml` and restarts Observer:

```toml
[sftp]
listen = "0.0.0.0:40126"
```

At that first start Observer makes the server's SSH host key and logs its fingerprint
(`SFTP listening ... fingerprint=SHA256:...`); clients compare it on their first connection. Expose the
port directly: behind a TCP proxy every client would share the proxy's address, which the lockouts and
the pipelines' allowed networks rely on. Allow TCP 40126 in the firewall only for the networks that
need it.

In the panel, under **Files**:

- **Groups** say what is reachable, folder by folder: allow or deny rules with rights (list, read,
  create, modify, rename, delete, make and remove folders), name filters such as `*.ytd`, and an
  optional largest file. A deny always wins. Each group shows its reach: **Read**, **Assets** (changes
  only textures, models, images, audio and fonts) or **Code** (anything else, which runs code on the
  host). Only the owner writes groups.
- **People** log in with their username and SSH keys they add themselves under **My account**
  (Ed25519 or ECDSA; `ssh-keygen -t ed25519` makes one). The owner, or someone with the permission to
  manage file access, gives them groups; Code groups stay the owner's to give.
- **Pipelines** log in as `+name` with a key Observer makes and shows once, valid 90 days by default
  and 365 at most, optionally from allowed networks only. Store the private key in the CI's secrets;
  a deploy step then runs, for example:

  ```sh
  sftp -b deploy.txt -P 40126 -i deploy.key -o StrictHostKeyChecking=yes +deploy-main@panel.example.com
  ```

Whatever the groups say, no one reaches the cfg files (`*.cfg`, which hold the licence key and the
database string), `cache/`, Observer's own folders and `observer.toml`, `.git`, `.env` files and key
files. Every change is in the audit log; uploads are written to a temporary file and swapped in only
when complete, so FXServer never reads half a file. Observer never restarts a resource after an
upload: restart it from the console, or let your own resource react.

## What changed since the last start

At each start of FXServer, Observer records what the server runs from: the build and the game build,
the launch settings, the cfg files `exec` reaches, and every resource under `resources/` with its
manifest's version, its git head, its npm packages and a digest of each file. The server never waits
for it: the folder is listed as FXServer boots, and the files that changed are read once the server
is online, at 64 MB/s at most. The first start reads everything once, in the background; later starts
read only what changed.

The **Changes** page compares the last two starts, any two, or the last start with the disk now
(**Check the disk**: what the next restart will apply). When a restart cut the previous start's reading
short, the last start is compared with the latest one that was read in full. It shows what txAdmin cannot: a resource whose
files changed while its version did not. Text files (Lua, JS, JSON, cfg, YAML and the like, up to
1 MiB) get line diffs; escrow-encrypted, binary and minified files are compared by size and hash.

- **Secrets.** Contents are masked before they are stored: convar secrets, webhook URLs, bot tokens,
  API keys, private keys, `Authorization` headers, credentials in connection strings and values given
  to names such as `password` or `token` (in code, JSON, YAML, TOML or XML) become `[REDACTED]`, and the diff only says when one changed. `.env`, key files and
  `txData` are never stored. A secret in a shape no rule knows stays visible to whoever may read
  contents, so the owner can hide more files with patterns and purge what a file left in the history.
- **Who sees what.** *View changes* shows the history, versions, files and counts; *Read changed
  files* adds the diffs and whole files and is critical: it is the server's code. Reading contents is
  in the audit log. The cfg files and the launch configuration stay the owner's alone, as they are in
  the launch settings.
- **Storage.** `changes.sqlite3` keeps starts 180 days and 2 GB of compressed text by default, the
  oldest contents dropped first; it is never in the daily backup, and a copy that cannot be read is set
  aside and started again, as the statistics are. Every file is read again once a week to catch a copy
  that kept both size and time.
- **For resources.** `observer:startChanges` tells your resources what changed once a start's files are
  read: post it to your staff's channel your way (see the contract).

## Server tools

Server tools look in the server's resources for problems FXServer does not report, and repair those
they can without changing what players see. The first one finds duplicate archetypes. A tool runs when
the owner asks, on the **Tools** page or from the [command line](#command-line), and before every
start of FXServer once the owner turns that on. Nothing is written into the server folder until the
owner runs a fix or turns fixes on.

### Duplicate archetypes

An archetype is the definition of a prop, a vehicle, a weapon or a ped model, which the FiveM client
registers by the hash of its name. When a resource defines one that is already registered, the client
logs, once per archetype and session:

```text
Duplicate Archetype 'prop_gift_box' (F032591F), seen in 'gift_props2.ytyp' and 'gift_props.ytyp'
```

The line goes to the client's log, on the player's machine: the server never sees it, and neither
does Observer's console. `Extra` in place of a file stands for a weapon, vehicle or ped `.meta`; the
same file named twice means two copies inside it.

**What it reads.** The cfg and the manifests, to predict which resources start and in which order;
the `.ytyp` files the started resources stream, one per name, as the client loads them; and the
`.meta` files they load as `WEAPON_METADATA_FILE`, `VEHICLE_METADATA_FILE` or `PED_METADATA_FILE`.
It needs no GTA V install and decrypts nothing: a file encrypted by the asset escrow is listed as
unreadable, and archives loaded as `RPF_FILE` are not opened. The archetypes of the game itself and
of Rockstar's updates are not read, so a resource that defines one of them again is not seen.

**What it fixes.** The copies of one archetype form a group. By default, the tool fixes a group, by
taking out every copy but one, only when all of these hold:

- the copies are identical, so which one the game uses cannot matter;
- they all lie in one resource, so the kept copy starts and stops with those taken out;
- that resource is not protected by the asset escrow (a `.fxap` at its root): its author ships the
  fix, and the report names the files and entries to pass on;
- none defines an interior (an MLO), and `.ytyp` copies do not meet `.meta` ones;
- no other file can take the place of a `.ytyp` involved: no other `.ytyp` of the same name in the
  `stream/` of any resource, its own included, started or not, and no other resource requesting or
  listing its name (`DLC_ITYP_REQUEST`, a `.ymf` manifest or `_cache_y.dat`); for copies in two
  files, no manifest of their resource lists them;
- every file can be changed: a layout the tool knows how to change, not read-only, not reached
  through a link, not in an excluded resource;
- the fix leaves exactly one copy: nothing is taken out of a group it cannot fully fix.

The kept copy is the last one registered. Every group the tool leaves is reported with every reason,
so the report says which setting would change the answer, or what to pass on to a resource's author.
With the default settings no fix depends on the prediction: it only orders the report and picks which
of identical copies stays.

**How it writes.** A fix takes the extra entries out of a `.ytyp`'s archetype list, or the extra
`Item`s out of a `.meta`, and leaves the rest of the file as it was. The new file is written under a
temporary name beside the old one, then renamed over it, never written in place: FXServer hard-links
every stream file into its `cache/`, and a write in place would change the cached copy too. A file
changed since the scan is left alone. A change takes effect the next time FXServer starts the
resource.

Each write is recorded before it starts: whatever stops Observer midway, its next start tells from the
file's contents whether the write landed, and removes a temporary file left behind. A run leaves
64 MiB free on the server folder's volume and on the data folder's, and stops at the first file that
would take more.

**Backups, heal and undo.**

- Before it first changes a file, Observer keeps it as it was in `data/tools/backups/<sha256>`, with
  `<sha256>.json` naming the server folder, the file and the time, so that it can be copied back by
  hand even without the database. A backup is verified before every restore, and kept while a fix
  relies on it and for 30 days after the last one ends.
- **Heal** runs before every start Observer makes of FXServer, whatever the settings and the cause
  of the start, an automatic restart after a crash included, and before every fix. It puts back
  every entry whose kept copy no longer stands in for it: its file gone or no longer defining the
  archetype, another file now able to take its place, its resource excluded, or escrowed while
  escrowed resources are skipped. When unsure, it puts the entry back: a duplicate warning is
  harmless, a missing prop is not. A file someone else has changed since is left as it is, and no
  longer counted as fixed. A scan heals nothing: it says how many files hold a fix.
- **Undo** puts back what fixes took out, by rebuilding each file from its backup: what one run took
  out, one file's entries, or everything. A file changed since Observer wrote it is left alone.

**Settings**, per server, on the **Tools** page or with `observer tools config`:

- **At start**: **Off** (the default), the tool runs only when asked; **Report**, it scans before
  every start and writes to the console when the result has changed; **Fix**, it scans and applies
  the fixes before every start. Heal runs whatever this says.
- **Differences**: **Identical only** (the default) fixes identical copies only; **Keep the latest
  copy** fixes copies that differ too, keeping the last one registered. The client keeps every copy,
  and which one the game uses when they differ is not known for sure: "the last one" is a
  prediction. If the game uses another, **Keep the latest copy** changes the model, the textures or
  the draw distance players see.
- **Escrowed resources**: **Skip** (the default) only reports escrowed resources; **Allow** fixes
  their plain files too.
- **Excluded resources**: resources never changed, by name as FXServer knows them, case included.
  Put there those a pipeline or a file group keeps: otherwise each deploy brings their duplicates
  back, and the next fix changes the file again.

Excluding a resource puts its entries back at the next heal, and so does setting **Escrowed
resources** back to **Skip** for escrowed resources.

The **Tools** page is the owner's. **Last run** shows the latest run and what it found or changed.
**Groups** lists every group: its copies, how they differ (only in draw distances and bounds, or in
more), what the tool does and why. **Files changed** lists the files the fixes changed, with their
backups. **Scan now** makes a new report and writes nothing, even while FXServer runs. **Fix**
applies the plan on screen: if a fresh scan plans something else, it applies nothing (the heal it
runs first may have put entries back) and the page shows the new plan instead. Once an undo or a
heal has put entries back, the page says the report on screen is out of date: scan again before
fixing. **Undo** puts back the entries taken out of one file, or by one run, in the folder they were
taken out of; **Undo all** those of every file in the folder of the launch settings. A change made
in a folder that is gone, or is another folder now, is listed without **Undo**: its backup puts it
back by hand. **Fix**, **Undo**, **Undo all** and the settings ask for the owner's password again,
and the three buttons run only while FXServer is stopped. Writes and settings changes are in the
audit log, those of a run that Observer's stop cut short at its next start; report runs are not.

Before a start, FXServer waits for the tool. Past 60 seconds, the tool is told to stop between two
files, and FXServer starts once it has; a stop in the meantime cancels the start. FXServer never
starts while a tool writes: a start asked during a fix or an undo waits for it to end, even past 60
seconds. On an automatic restart after a crash, the tool only heals. Its console lines appear under
`tools`, a summary and at most 10 more, and the **Tools** page has the rest; a report, or a fix that
wrote nothing, says nothing when its result is that of the last report. Uploads, renames and
deletions over SFTP wait while a tool writes. `observer fxserver run` runs no tool, not even the
heal, and says so.

From the host:

```sh
observer tools run duplicate-archetypes [--fix] [--json] [--data DIR] [--cfg FILE | --all-resources]
                                        [--differences D] [--escrowed E]
observer tools undo duplicate-archetypes --run ID | --all [--data DIR]
observer tools config duplicate-archetypes [--at-start M] [--differences D] [--escrowed E]
                                           [--exclude NAME]... [--include NAME]...
```

- `run` reads the server folder and the cfg of the launch settings, or the folder `--data` names
  with the cfg `--cfg` names in it (`server.cfg` by default); `--all-resources` reads no cfg and
  takes every resource FXServer accepts as started, in name order, and still reads the cfg for the
  game it runs (without one, the server is taken to run GTA V). `--differences` and `--escrowed`
  change the saved settings for this run only, but `--fix` takes `--escrowed allow` only once it is
  saved with `config`: the next heal would put back what the fix took out of escrowed resources while
  the saved setting skips them. It prints the report, as JSON with `--json`.
- Without `--fix`, `run` writes nothing, the database included: it heals nothing, and its report is
  not in the **Tools** page's **Last run**. It needs no database either: with `--data`, it works
  before Observer's first start, with the default settings, and so it does with a database this
  build cannot read yet, before its first start after an upgrade, as it says.
- With `--fix`, `run` applies the plan and prints what became of each file, and the undo that puts
  it back, with the run's number and the `--data` it was given. The run is recorded and audited like
  a fix from the **Tools** page.
- `run` exits with 0 when no duplicate is left once it ends, and with 2 when some are: for a report,
  every group it finds, fixable or not, since it fixes none; with `--fix`, every group it did not
  fix. It exits with 1 when it cannot run, or when a fix stops before the end, such as on a full
  volume, once it has printed what it wrote. A mistyped command also exits with 2, and prints its
  usage.
- `undo --run ID` puts back what that run took out, in the folder it changed, as long as that still
  is the same folder; `undo --all` puts back everything in the folder of the launch settings.
  `--data DIR` undoes in that folder instead, for either.
- `config` prints the settings and changes those its options name: `--at-start` (`off`, `report` or
  `fix`), `--differences` (`identical` or `keep-latest`), `--escrowed` (`skip` or `allow`),
  `--exclude NAME` to add a resource to the excluded ones and `--include NAME` to take one off, both
  repeatable. A change takes effect at the next start.
- `undo` and `config` exit with 0, or 1 on an error.

`run` without `--fix` takes no lock, and `config` checks that no one changed the settings meanwhile:
both work while Observer runs. `--fix` and `undo` take Observer's lock, so stop Observer first, or
use the **Tools** page.

**Limits.**

- **Runtime starts.** Resources that scripts or the console start, stop or restart while the server
  runs are not seen, nor constraints such as `/assetpacks` that only FXServer decides: the report may
  list groups the clients never meet, or miss some. No fix depends on them, since the copies of a
  fixed group share one resource.
- **RedM** is not supported: when the cfg sets `gamename rdr3`, a scan or a fix is refused, with
  `--all-resources` too, and the next start puts back whatever an earlier fix took out. An undo
  still works.
- **File metadata.** A replaced file is a new file: on Linux it keeps its mode, not its ACLs or
  extended attributes; on Windows it takes the permissions its folder gives new files. A read-only
  file is never changed.
- **Folder order on Linux.** Windows lists folders in a set order, Linux in none: where a resource
  holds two stream files of the same name, which one FXServer sends may differ from the prediction.
  The report notes it, and such files are never changed.

## Turning features off

File access over SFTP (`sftp`), file changes (`changes`), the server tools (`tools`), the diagnostics
(`diagnostics`), the statistics (`metrics`), the players (`players`) and the player drops (`drops`) can
each be turned off. A feature that is off runs none of its code: Observer does not open its port, start
its tasks or open its database, and its API routes answer `feature_disabled`. What it recorded stays on
disk, and its retention waits until it runs again. Supervision, the console, accounts, the audit log,
the restart schedule and FXServer's builds are always on.

`GET /api/features` (**View settings**) says for each feature whether it runs, what holds it off, and
what it opens: its port, its tasks, its files, its routes. Only the owner turns one on or off, with
`PUT /api/features/{feature}` and the password again; each change is in the audit log.

- **Off applies at once.** By the time the API answers, the port is closed, its sessions have ended and
  the feature's tasks have stopped. Turning the players off turns the drops off too.
- **On applies at the next start of Observer,** which restarts FXServer, except for the server tools,
  which apply from the next start of FXServer. A port only opens when the host restarts Observer.
- **The tools' heal goes on.** While a fix of the server tools is applied, the work before each start
  still puts back what it took out once the copy it kept is gone, even with the tools off: undo the
  fixes first to turn the tools off entirely.

The host can keep features off whatever the panel says, the owner's account included, in
`observer.toml`; `"off"` is the only value:

```toml
[features]
sftp = "off"
diagnostics = "off"
```

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
- **SFTP: folders are not renamed or moved,** files are; every `*.cfg` file is refused until Observer
  reads which ones the server executes; a file FXServer holds open on Windows cannot be replaced until
  it lets go of it.
- **Changes: who changed a file** is told only for SFTP writes, by path and time: a hint, since a file
  can be written again after. Changes made while the server runs show at the next start or with
  **Check the disk**, not as they happen.

## Licence

No licence has been chosen yet. Until one is, all rights are reserved. The third-party software Observer
ships is listed with its licences in `THIRD-PARTY-LICENCES.txt` and on the panel's **About** page.
