# Integration contract

Observer never shows anything to players. It tells your resources what is about to happen to the
server, with data and no text, and your resources decide what players see, what gets saved and when.
This page is that contract: the server events Observer raises, what they carry, and what you can rely
on.

## The bridge

The bridge is a server-side resource that Observer writes into the build's `system_resources` folder
before every start of FXServer, and ensures before your cfg runs. Observer overwrites any change made
to it. It reports the server's health to Observer and turns Observer's commands into server events.
It has no client script and no UI.

Its name changes with every start of FXServer: Observer draws it at random, so that no script can
target the bridge by a name known in advance. Read it from the `observer_bridge` convar, which
Observer sets on FXServer's command line with `+set`: a server convar, never sent to clients and
never listed in `/info.json`. Do not ensure the bridge yourself, and never write its name into a cfg
or a script: the next start gives it another.

```lua
local bridge = GetConvar('observer_bridge', '') -- empty when Observer did not start the server
```

## Version

This is contract version **2**.

```lua
local version = exports[GetConvar('observer_bridge', '')]:getContractVersion() -- 2
```

Within a version, fields and `reason` values are only ever added. Ignore fields you do not know, and
treat a `reason` you do not know as the closest one you do (`stop` for `observer:shuttingDown`). A
field renamed or removed, or a change of meaning, is a new version.

Version 2 changed how a resource finds the bridge: version 1 named it `observer`, version 2 draws its
name at each start and gives it in `observer_bridge`. The events and their payloads did not change.

## Events

Every event is a server event raised with `TriggerEvent`, with one table as its argument. Times are
integer Unix milliseconds, UTC. `by` is the username of whoever acted in the panel, `observer`
for what Observer does by itself, or `local` for someone at the host's own status window (Windows),
who signs in to nothing. No account can take either name.

| Event | When |
|---|---|
| `observer:restartScheduled` | At each warning before a restart, and at once for the nearest warning when a restart is planned or moved inside the warning window. |
| `observer:restartSkipped` | A restart announced since this start of FXServer will not happen. |
| `observer:restartRescheduled` | A restart announced since this start of FXServer moved to another time. |
| `observer:shuttingDown` | FXServer quits in `delayMs`: save now. |
| `observer:startChanges` | Once per start of FXServer, when its files are hashed: what changed since the start before. |

### `observer:restartScheduled`

| Field | Type | Meaning |
|---|---|---|
| `at` | integer | When the restart happens. |
| `secondsRemaining` | integer | Seconds left until `at`, rounded to the nearest second. Equal to the warning's minutes × 60 when sent on time; the real time left after a wake from sleep or a restart planned a few minutes ahead. |
| `reason` | string | `schedule` for the server's restart schedule, `oneOff` for a single restart someone planned. |

The warnings are the schedule's (30, 15, 10, 5, 4, 3, 2 and 1 minutes by default), and the owner can
change them.

### `observer:restartSkipped`

| Field | Type | Meaning |
|---|---|---|
| `at` | integer | The restart that will not happen, as announced. |
| `by` | string | Who skipped it, cancelled it or changed the schedule; `observer` when Observer could not make it. |
| `reason` | string | The source of the skipped restart: `schedule` or `oneOff`. |

### `observer:restartRescheduled`

| Field | Type | Meaning |
|---|---|---|
| `at` | integer | When the restart happens now. |
| `previousAt` | integer | The time announced before. |
| `by` | string | Who delayed it, planned a single restart before it, or restored a skipped restart that now comes first. |
| `reason` | string | The source of the restart at `at`: `schedule` or `oneOff`. |

An `observer:restartScheduled` for the new time follows at once when it is already inside the warning
window.

### `observer:shuttingDown`

| Field | Type | Meaning |
|---|---|---|
| `delayMs` | integer | Milliseconds until Observer sends `quit` (5,000 by default, 0 when Windows closes Observer's console). |
| `reason` | string | `stop`, `restart` (asked in the panel or the host's window), `scheduled` (the schedule or a single restart someone planned), or `shutdown` (Observer itself is stopping). |
| `by` | string | Who asked, `local`, or `observer`. |

### `observer:startChanges`

Raised once per start of FXServer, after the server is online and Observer has read the files that
changed, even when nothing did. The owner can turn it off.

| Field | Type | Meaning |
|---|---|---|
| `at` | integer | When this start's snapshot was taken. |
| `previousAt` | integer, or nil | When the start it compares with was taken; nil for a baseline. |
| `baseline` | boolean | The first snapshot of the server, which compares with nothing: no resource is listed. |
| `build` | table, or nil | `{ from, to }`: the FXServer builds, when it changed. |
| `gameBuild` | table, or nil | `{ from, to }`: the enforced game builds, when it changed. |
| `counts` | table | `added`, `removed`, `updated` (files and version changed), `sameVersion` (files changed, version did not), `renamed`, `moved` (another category), `didNotStart` (started last time, not this time), `pending` (files not all read yet, left out of `resources`). |
| `resources` | array | One table per resource: `name`, `change` (`added`, `removed`, `updated`, `sameVersion`, `renamed`, `moved` or `didNotStart`), `from` and `to` (the manifest's versions, when it has them), `fromHidden` and `toHidden` (true when a version was left out, see below). Added first, then updated, then changed under the same version, then the rest. |
| `skipped` | integer | Resources left out because of their name. |
| `truncated` | boolean | Resources left out to keep the event within what the console takes: the panel lists them all. |

Every name and version in the event is safe to put in a console command: a name that is not 1 to 64
letters, digits, `_`, `.` or `-` is counted in `skipped` and never listed, and a version that is not 1
to 32 of those or `+` and `~` is left out, with `fromHidden` or `toHidden` set. Manifests are written
by third parties. Nothing else of a file's name or contents is ever sent.

## Ordering and delivery

- Observer is the only writer, so the events of one run of FXServer arrive in the order they were
  raised.
- Nothing is queued for a server that is still starting, and nothing is replayed to the next run. A
  resource restarted in the middle of a run has missed what came before: rely on `at`, not on having
  seen every warning.
- For one `at`, at most one `observer:restartScheduled` per warning, with `secondsRemaining`
  decreasing. After a sleep or a clock change, only the nearest warning goes out, never several at
  once.
- `observer:restartSkipped` and `observer:restartRescheduled` only name an `at` announced in the same
  run. Should that restart come back (restored, or the schedule turned on again), it is announced
  again, from its nearest warning.
- A restart that happens is always preceded by `observer:shuttingDown`, and no `observer:restart*`
  event follows it. A second `observer:shuttingDown` may follow when a stop, or Observer stopping,
  overtakes a restart: the latest one says what happens.
- Nothing comes after `quit`.
- Delivery is best effort. A main loop that is stuck gets an event late, or never.
- `observer:startChanges` comes once a run, never for a run that ended before its files were read.
- An announced restart can also vanish without `observer:restartSkipped` when the server stops or
  crashes first: the run ends anyway.

## Security

The bridge raises these events only for commands typed on the server console, where only Observer
writes, and never for a call from another resource. Listen with `AddEventHandler` only. Never
`RegisterNetEvent` these names: clients could then raise them.

The drawn name keeps away scripts written against a fixed name, and no resource can take it before
the bridge starts. It is no secret from the server's own resources: any of them can list the
resources, read `observer_bridge` and the bridge's files. Run only server resources you trust.

## txAdmin's events

Resources written for txAdmin can keep working: with **Send txAdmin's events too** checked in the
launch settings, the bridge also raises txAdmin's events, from the next start of FXServer. Each comes
right after Observer's own event. `translatedMessage` and `message` are always empty: Observer writes
no text.

| Observer's event | txAdmin's events, in order | Their payload |
|---|---|---|
| `observer:restartScheduled` | `txAdmin:events:scheduledRestart` | `secondsRemaining` rounded up to whole minutes, as txAdmin counts them (166 becomes 180), `translatedMessage = ''` |
| `observer:restartSkipped` | `txAdmin:events:scheduledRestartSkipped`, then the deprecated `txAdmin:events:skippedNextScheduledRestart` | `secondsRemaining` (until `at`), `temporary` (true for `oneOff`), `author` (`by`); the deprecated one without `author` |
| `observer:restartRescheduled` | none | |
| `observer:shuttingDown` | `txAdmin:events:serverShuttingDown` | `delay` (`delayMs`), `author` (`by`), `message = ''` |

## Not in version 1

Exports to read the next restart or the server's state, and a way for a resource to send something to
Observer, come later.

## Example

[`observer_example`](observer_example) finds the bridge and checks the contract version when it
starts, prints each event to the server console with its fields, and marks with short comments where
a team plugs in its notifications, its cancellation notice and its saves. It shows nothing to
players. Copy what fits into your own resources rather than running it as is.
