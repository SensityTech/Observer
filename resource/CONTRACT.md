# Integration contract

Observer never shows anything to players, but for one text: the refusal message the owner writes
for bans. It tells your resources what is about to happen to the server, with data and no text, and
your resources decide what players see, what gets saved and when. This page is that contract: the
server events Observer raises, what they carry, how your resources hold a stop until they have
saved, what they can tell Observer about players, how bans keep players out, and what you can rely
on.

## The bridge

The bridge is a server-side resource that Observer writes into the build's `system_resources` folder
before every start of FXServer, and ensures before your cfg runs. Observer overwrites any change made
to it. It reports the server's health to Observer, turns Observer's commands into server events, and refuses
banned players. While someone has the panel's live map open, it also reads where each player stands,
from OneSync, every 1 to 5 seconds, and reports it to Observer; it raises no event for it and stops
within 30 seconds after the last one closes the map. It has no client script and no UI.

Its name changes with every start of FXServer: Observer draws it at random, so that no script can
target the bridge by a name known in advance. Read it from the `observer_bridge` convar, which
Observer sets on FXServer's command line with `+set`: a server convar, never sent to clients and
never listed in `/info.json`. Do not ensure the bridge yourself, and never write its name into a cfg
or a script: the next start gives it another.

```lua
local bridge = GetConvar('observer_bridge', '') -- empty when Observer did not start the server
```

## Version

This is contract version **7**.

```lua
local version = exports[GetConvar('observer_bridge', '')]:getContractVersion() -- 7
```

Within a version, fields and `reason` values are only ever added. Ignore fields you do not know, and
treat a `reason` you do not know as the closest one you do (`stop` for `observer:shuttingDown`). A
field renamed or removed, or a change of meaning, is a new version. So is a new export of the
bridge, so that a resource can check for it before calling it.

Version 7 added holding a stop: `holdShutdown` and `releaseShutdown`, and `maxDelayMs` in
`observer:shuttingDown`. A resource that never calls them sees a stop as it was before. Version 4
added bans: the bridge holds every connecting player until Observer checks them, refuses a
banned one, drops a connected player when they are banned, and raises `observer:playerRefused` and
`observer:playerBanned`. Version 3 added the player labels: `setPlayerLabel` and `clearPlayerLabel`. Version 2 changed how a
resource finds the bridge: version 1 named it `observer`, version 2 draws its name at each start and
gives it in `observer_bridge`. The events and their payloads did not change.

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
| `observer:shuttingDown` | FXServer is about to quit: save now. See [Holding a stop](#holding-a-stop). |
| `observer:startChanges` | Once per start of FXServer, when its files are hashed: what changed since the start before. |
| `observer:playerRefused` | A ban refused a connecting player. See [Bans](#bans). |
| `observer:playerBanned` | A ban is about to drop a connected player. See [Bans](#bans). |

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
| `delayMs` | integer | Milliseconds until Observer sends `quit` when no resource holds the stop (5,000 by default, 0 when Windows closes Observer's console). |
| `maxDelayMs` | integer | Milliseconds until Observer sends `quit` whoever still holds the stop: the owner's limit, 60,000 by default. Never less than `delayMs`, and equal to it when nothing can hold this stop. |
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
  overtakes a restart: the latest one says what happens, and `delayMs` and `maxDelayMs` give what
  is left of the first one's.
- `quit` comes once `delayMs` has passed and no resource holds the stop, and at `maxDelayMs` at the
  latest. Nothing comes after `quit`.
- Delivery is best effort. A main loop that is stuck gets an event late, or never.
- `observer:startChanges` comes once a run, never for a run that ended before its files were read.
- An announced restart can also vanish without `observer:restartSkipped` when the server stops or
  crashes first: the run ends anyway.

## Holding a stop

A resource that saves when the server stops can hold the stop until it has saved. Ask once, as your
resource starts, and release each stop once your saves are done:

```lua
local bridge = GetConvar('observer_bridge', '')
exports[bridge]:holdShutdown()

AddEventHandler('observer:shuttingDown', function(payload)
    -- your saves go here; they may wait on your database
    exports[bridge]:releaseShutdown()
end)
```

From `holdShutdown()` on, every stop Observer announces waits for your resource: Observer sends
`quit` once `delayMs` has passed and every resource that holds the stop has released it. A resource
that never asks changes nothing: without one, `quit` comes after `delayMs`, as it did before
version 7.

- The wait has a limit, which the owner sets in the panel: 60 seconds by default, from 5 seconds to
  10 minutes. `maxDelayMs` gives it. Past it Observer sends `quit` anyway, and names the resources
  that had not released the stop in the console and the audit log. While it waits, the panel names
  them too, and whoever may stop the server there can have FXServer quit without waiting for them:
  `delayMs` still runs to its end.
- Both exports return `true`, or `false` and why. `holdShutdown`: `tooMany` (64 resources hold the
  stop already) or `invalid` (not called by a resource). `releaseShutdown`: `notStopping` (no stop
  is under way: the hold stays) or `notHeld` (your resource does not hold the stop).
- Release in every case, a failed save included: a resource that never releases costs every stop
  the whole limit, and is named for it.
- A hold lasts until your resource stops. Started again, it asks again as it starts. A resource
  that stops during a stop is no longer waited for.
- `holdShutdown()` called once a stop is under way counts as well, as long as it comes before
  `quit`. Ask as your resource starts all the same: the stop is then held from its first instant,
  even on a server too busy to run your handler within `delayMs`.
- The bridge keeps the holds. Should someone stop or restart the bridge itself, they are gone until
  each resource asks again.

What waits for the resources that hold a stop, and what does not:

| What ends the run | Does it wait? |
|---|---|
| A stop or a restart, from the panel or the host's window | Yes. |
| A restart of the schedule, a single restart someone planned | Yes. The restart's time is when `observer:shuttingDown` is raised; FXServer is back that much later. |
| Observer installing its own update, a build of FXServer applied by a restart, a return to the last healthy build | Yes: each is a stop or a restart like any other. |
| Observer stopped by Ctrl+C, a signal or its service | Yes. Under systemd, Observer asks for the time it waits, which takes systemd 236 or later; anything else that stops Observer sets its own limit. |
| A second stop that overtakes the first | It is still the same stop: a release already given stands, and the limit counts from the first event. |
| Windows closing Observer's console, logging off or shutting down | No: Windows ends Observer within seconds. `delayMs` and `maxDelayMs` are 0, and a wait under way is cut short. |
| A crash, or a server that stopped responding | No: no event is raised, and there is no one to wait for. Save as you go, not only at the stop. |
| `quit` typed in the console | No: FXServer quits at once. |
| A stop while FXServer is still starting | No: the bridge has not reported yet, and nothing is raised. |

A server whose main loop is stuck when the stop begins runs no handler. The resources that hold the
stop are then waited for until the limit, FXServer is told to quit, and 15 seconds later its
processes are killed.

The backup Observer makes of the game's database before a restart runs once FXServer has exited: it
holds what your resource saved before it released the stop, and may miss what a resource cut off at
the limit was still saving.

## Player labels

Your resources can give each connected player a label, which Observer's panel shows beside their
FiveM name: a roleplay character's name, an id of your own. Players never see it.

```lua
local bridge = GetConvar('observer_bridge', '')
local ok, refused = exports[bridge]:setPlayerLabel(source, { name = 'John Doe', id = 'CHAR-1234' })
exports[bridge]:clearPlayerLabel(source)
```

| Field | Type | Meaning |
|---|---|---|
| `name` | string, or nil | What the player is called on your server, up to 64 characters. |
| `id` | string, or nil | An id of your own, up to 32 characters. |

- Give at least one of the two. Each call replaces the whole label. Spaces around a text are dropped,
  and a blank text counts as nil.
- Both exports return `true`, or `false` and why: `notConnected` (no such player; call it once the
  player has joined, from `playerJoining` on), `invalid` (not a table, a field that is not a string,
  or neither field), `tooLong`.
- A label lasts as long as the player's session: it goes when they leave or the server stops, and
  nothing gives it back at the next start. Give it again when they join again.
- The bridge sends labels to Observer once a second, the latest of each player: setting one in a
  loop sends no more than that.
- Observer notes which resource gave a label, and the panel shows it.

What Observer does with a label:

- It is display data. Observer never compares it with anything and never uses it to find, link,
  merge or rename a player: who a player is stays what FXServer confirms, their licence.
- Control characters, bidirectional overrides and invisible characters are removed, and the lengths
  are checked again. A field past its length is left out.
- The panel shows a label only once the player's identity is confirmed. Observer keeps each label a
  player had on their profile, with when it was first and last given, as long as it keeps their
  names. The Players page finds a player by the start of a label's name, or by a label's whole id,
  in any case.

## Bans

The staff bans players in Observer's panel. A ban holds the player's licence, their other
identifiers and their hardware tokens, for a time or for good. The bridge enforces it:

- On `playerConnecting`, the bridge defers the connection and asks Observer. Observer answers within
  milliseconds. Your own `playerConnecting` handlers run as they do without Observer: the player
  gets in once every handler is done.
- A banned player is refused with the refusal message the owner wrote in the panel, which can name
  the reason, when the ban ends and its code. It is the only text Observer shows players.
- Without an answer within 5 seconds, the player gets in. Observer checks every player again once
  they joined, and drops a banned one with the same message.
- When the staff bans a connected player, the bridge drops them at once with that message.

The bridge raises one event for each, for your logs or your staff's channel. Players have already
been refused, or are about to be dropped: nothing a resource does changes it.

### `observer:playerRefused`

| Field | Type | Meaning |
|---|---|---|
| `ban` | string | The ban's code, `K7QM-2XPA`, which the player can quote to appeal. |
| `reason` | string | The reason the staff gave. |
| `endsAt` | integer, or nil | When the ban ends; nil for a permanent ban. |
| `by` | string | Who banned the player. |
| `name` | string, or nil | The player's name, as FiveM gives it. |
| `license` | string, or nil | The player's licence, `license:` and 40 hex digits. |

### `observer:playerBanned`

| Field | Type | Meaning |
|---|---|---|
| `player` | integer | The player's server id, dropped right after the event. |
| `ban` | string | The ban's code. |
| `reason` | string | The reason the staff gave. |
| `endsAt` | integer, or nil | When the ban ends; nil for a permanent ban. |
| `by` | string | Who banned the player. |
| `name` | string, or nil | The player's name. |
| `license` | string, or nil | The player's licence. |

## Security

The bridge raises these events, refuses players and drops them only for commands typed on the server
console, where only Observer writes, and never for a call from another resource. Listen with `AddEventHandler` only. Never
`RegisterNetEvent` these names: clients could then raise them.

Any server resource can give a label: the exports are for your resources, which can already do
anything on the server. Never call them with text a client sent you without checking it first.

Any server resource can hold a stop, too. The owner's limit bounds what it can cost, and Observer
names every resource that holds one.

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

txAdmin has no way to hold a stop: a resource written for it gets `delay`, and saves within it.

## Not in version 7

Exports to read the next restart or the server's state come later.

## Example

[`observer_example`](observer_example) finds the bridge and checks the contract version when it
starts, prints each event to the server console with its fields, and marks with short comments where
a team plugs in its notifications, its cancellation notice and its saves. It holds each stop for as
long as its convar `observer_example_save_ms` says, 0 by default, as a save would. Its console
command `observer_example_label` gives a player a label, or clears it, to try the panel's side. It shows
nothing to players. Copy what fits into your own resources rather than running it as is.
