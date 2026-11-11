# Integration contract

Observer never shows anything to players, but for what the owner writes: the refusal messages, one
for bans, one for maintenance mode, one for the allowlist, the kick message, and the name the server
takes in FiveM's server list during maintenance. It tells your resources what is about to happen to
the server, with data and no text of its own, and your resources decide what players see, what gets
saved and when. This page is that contract: the server events Observer raises, what they carry, what
your resources can tell Observer about players, how bans, maintenance mode and the allowlist keep
players out, how the staff's kicks, announcements and messages reach your resources, and what you
can rely on.

## The bridge

The bridge is a server-side resource that Observer writes into the build's `system_resources` folder
before every start of FXServer, and ensures before your cfg runs. Observer overwrites any change made
to it. It reports the server's health to Observer, turns Observer's commands into server events, drops
the players the staff kicks, and refuses the players Observer keeps out: banned ones, everyone but a
short list during maintenance, and those the staff has not approved while the allowlist is on. While
someone has the panel's live map open, it also reads where each player stands,
from OneSync, every 1 to 5 seconds, and reports it to Observer; it raises no event for it and stops
within 30 seconds after the last one closes the map. In the same way, while someone looks at the
server's entities in the panel, it lists the vehicles, peds and objects OneSync knows, every 5 to 60
seconds, a millisecond of the main thread at a time, and deletes the ones the staff picks there: never
a player's own ped, a vehicle a player sits in or a train. It raises no event for either. It has no
client script and no UI.

Its name changes with every start of FXServer: Observer draws it at random, so that no script can
target the bridge by a name known in advance. Read it from the `observer_bridge` convar, which
Observer sets on FXServer's command line with `+set`: a server convar, never sent to clients and
never listed in `/info.json`. Do not ensure the bridge yourself, and never write its name into a cfg
or a script: the next start gives it another.

```lua
local bridge = GetConvar('observer_bridge', '') -- empty when Observer did not start the server
```

## Version

This is contract version **6**.

```lua
local version = exports[GetConvar('observer_bridge', '')]:getContractVersion() -- 6
```

Within a version, fields and `reason` values are only ever added. Ignore fields you do not know, and
treat a `reason` you do not know as the closest one you do (`stop` for `observer:shuttingDown`), and
a `cause` you do not know as a refusal you have no details for. A field renamed or removed, or a
change of meaning, is a new version. So is a new export of the
bridge, so that a resource can check for it before calling it.

Version 6 added the allowlist and maintenance mode, with `observer:maintenance`, and changed what
`observer:playerRefused` means: it is raised for every player refused at connection, says what
refused them in `cause`, and carries `ban`, `reason`, `endsAt` and `by` for a ban only.
Version 5 added what the staff does to connected players: the bridge drops the players the staff
kicks, with the kick message the owner wrote, and raises `observer:playerKicked`,
`observer:announcement` and `observer:directMessage`; with txAdmin's events, it also tells of each
ban made or lifted.
Version 4 added bans: the bridge holds every connecting player until Observer checks them, refuses a
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
| `observer:shuttingDown` | FXServer quits in `delayMs`: save now. |
| `observer:startChanges` | Once per start of FXServer, when its files are hashed: what changed since the start before. |
| `observer:playerRefused` | A connecting player was refused, by a ban, by maintenance mode or by the allowlist. See [Bans](#bans), [Maintenance mode](#maintenance-mode) and [Allowlist](#allowlist). |
| `observer:playerBanned` | A ban is about to drop a connected player. See [Bans](#bans). |
| `observer:playerKicked` | The staff kicked a connected player, who is dropped next. See [Kicks](#kicks). |
| `observer:announcement` | The staff wrote an announcement for the whole server. See [Announcements and messages](#announcements-and-messages). |
| `observer:directMessage` | The staff wrote a message for one connected player. See [Announcements and messages](#announcements-and-messages). |
| `observer:maintenance` | Maintenance mode was turned on or off, and once more at each start of FXServer while it is on. See [Maintenance mode](#maintenance-mode). |

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
- `observer:maintenance` says how things stand: the latest one of a run tells whether maintenance is
  on. A run that hears none started with maintenance off.

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
  the reason, when the ban ends and its code.
- Without an answer within 5 seconds, the player gets in. Observer checks every player again once
  they joined, and drops a banned one with the same message.
- When the staff bans a connected player, the bridge drops them at once with that message.

The bridge raises one event for each, for your logs or your staff's channel. Players have already
been refused, or are about to be dropped: nothing a resource does changes it.

### `observer:playerRefused`

Raised for every player refused at connection, whatever refused them: `cause` says what, and which
of the other fields come with it.

| Field | Type | Meaning |
|---|---|---|
| `cause` | string | What refused the player: `ban`, `maintenance` or `allowlist`. |
| `name` | string, or nil | The player's name, as FiveM gives it. |
| `license` | string, or nil | The player's licence, `license:` and 40 hex digits. |
| `ban` | string | For a ban: its code, `K7QM-2XPA`, which the player can quote to appeal. |
| `reason` | string | For a ban: the reason the staff gave. |
| `endsAt` | integer, or nil | For a ban: when it ends; nil for a permanent ban. |
| `by` | string | For a ban: who banned the player. |
| `request` | string, or nil | For the allowlist: the ID of the player's request, `7KQ-2XP`, which they quote to the staff; nil for a player who shows no licence. |

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

## Kicks

The staff kicks a connected player in Observer's panel, with a reason, or everyone at once. The
bridge drops them with the kick message the owner wrote in the panel, which can name the reason and
who kicked. A kick keeps no one out: the player can connect again at once. A kick of everyone drops
the players `GetPlayers()` lists: one who is still connecting is not among them.

The bridge raises `observer:playerKicked` once for each player of a kick, then drops them. Every
player of the kick is still connected while the handlers run: read what you need of a player, their
name, their identifiers, your own data, before your handler returns. Nothing a resource does stops
the kick.

### `observer:playerKicked`

| Field | Type | Meaning |
|---|---|---|
| `player` | integer | The player's server id, dropped once the events of the kick are raised. |
| `reason` | string | The reason the staff gave. |
| `by` | string | Who kicked the player. |
| `cause` | string | `kick` when the staff kicked this player, `kickAll` when they kicked everyone. Treat a cause you do not know as `kick`. |

## Announcements and messages

The staff writes an announcement for the whole server, or a message for one connected player, in
Observer's panel. Observer shows players nothing of either: the bridge raises an event with the
text, and your resources show it, in the chat, as a notification, or the way your server speaks to
its players. Without a resource that handles the event, nobody reads the text.

### `observer:announcement`

| Field | Type | Meaning |
|---|---|---|
| `message` | string | The text, up to 400 characters, on one line or several. |
| `by` | string | Who wrote it. |

### `observer:directMessage`

| Field | Type | Meaning |
|---|---|---|
| `player` | integer | The server id of the player the message is for. They were connected when the staff sent it and may have left since. |
| `message` | string | The text, up to 400 characters, on one line or several. |
| `by` | string | Who wrote it. |

## Maintenance mode

The staff closes the server to players from the panel while they work on it, and opens it again. It
stays closed through restarts of FXServer and of Observer, until someone ends it:

- On `playerConnecting`, once no ban refuses them, every player is refused with the maintenance
  message, which the owner writes in the panel. The bridge raises `observer:playerRefused` with
  `cause = 'maintenance'`.
- The owner keeps a list of players who still get in, by licence. They get in whether the allowlist
  has them or not; a ban still refuses them.
- Without an answer within 5 seconds, the player gets in, as with bans. Observer drops no one for
  maintenance: turning it on leaves the connected players in until they leave.
- The owner can give the server another name for FiveM's server list while maintenance is on. The
  bridge then sets `sv_hostname`, and `sv_projectName` when the server has one, to that name, and
  gives both their own value back when maintenance ends. A resource that reads either convar in
  the meantime reads the maintenance name. The bridge keeps what they held in server convars of its
  own, `observer_name_kept`, `observer_name_hostname` and `observer_name_project`: leave them
  alone.

### `observer:maintenance`

Raised when someone turns maintenance on or off, and once more at each start of FXServer while it is
on, when the server is online: a resource started with the server hears how things stand.

| Field | Type | Meaning |
|---|---|---|
| `on` | boolean | Whether the server is closed for maintenance now. |
| `at` | integer | When it was turned on, or off. The same for every event of one maintenance. |
| `by` | string | Who turned it on, or off. |

## Allowlist

While the owner has the allowlist on, only the players the staff approved get in. Observer goes by
the licence alone:

- On `playerConnecting`, once no ban and no maintenance refuses them, a player whose licence is not
  approved is refused with the allowlist's message, which the owner writes in the panel. The bridge
  raises `observer:playerRefused` with `cause = 'allowlist'`.
- The refusal makes a request, which the message names by its ID. The staff approves or denies it
  in the panel. A player who tries again keeps their request and its ID; one who was denied makes a
  new request, under a new ID.
- The staff can approve a licence before its player ever connects.
- A player who shows no licence, as on a server with `sv_lan`, cannot be approved: they are refused
  and make no request.
- Without an answer within 5 seconds, the player gets in, as with bans. Observer drops no one for
  the allowlist: turning it on, or taking an approval back, leaves the connected players in until
  they leave.

## Security

The bridge raises these events, refuses players and drops them only for commands typed on the server
console, where only Observer writes, and never for a call from another resource. Listen with `AddEventHandler` only. Never
`RegisterNetEvent` these names: clients could then raise them.

The text of an announcement or a message is what a member of the staff typed, without control or
invisible characters and nothing more: escape it for whatever shows it, as you would a player's
chat line.

Any server resource can give a label: the exports are for your resources, which can already do
anything on the server. Never call them with text a client sent you without checking it first.

The drawn name keeps away scripts written against a fixed name, and no resource can take it before
the bridge starts. It is no secret from the server's own resources: any of them can list the
resources, read `observer_bridge` and the bridge's files. Run only server resources you trust.

## txAdmin's events

Resources written for txAdmin can keep working: with **Send txAdmin's events too** checked in the
launch settings, the bridge also raises txAdmin's events, from the next start of FXServer. Each comes
right after Observer's own event, but for a ban's: txAdmin tells of a ban once, when it is made, and
so does the bridge, before the players the ban drops hear `observer:playerBanned`. A ban lifted
raises only txAdmin's event: Observer has none of its own for it. The `translatedMessage` of a
restart warning and the `message` of a shutdown are always empty: Observer writes no text.

| Observer's event | txAdmin's events, in order | Their payload |
|---|---|---|
| `observer:restartScheduled` | `txAdmin:events:scheduledRestart` | `secondsRemaining` rounded up to whole minutes, as txAdmin counts them (166 becomes 180), `translatedMessage = ''` |
| `observer:restartSkipped` | `txAdmin:events:scheduledRestartSkipped`, then the deprecated `txAdmin:events:skippedNextScheduledRestart` | `secondsRemaining` (until `at`), `temporary` (true for `oneOff`), `author` (`by`); the deprecated one without `author` |
| `observer:restartRescheduled` | none | |
| `observer:shuttingDown` | `txAdmin:events:serverShuttingDown` | `delay` (`delayMs`), `author` (`by`), `message = ''` |
| `observer:playerKicked` | `txAdmin:events:playerKicked` | `target` (`player`), `author` (`by`), `reason`, `dropMessage` (what the player reads). For a kick of everyone, once, after the last player's event and with nobody connected too, with `target = -1`, as txAdmin does |
| `observer:announcement` | `txAdmin:events:announcement` | `message`, `author` (`by`) |
| `observer:directMessage` | `txAdmin:events:playerDirectMessage` | `target` (`player`), `author` (`by`), `message` |
| `observer:playerRefused` | none | |
| `observer:playerBanned` | `txAdmin:events:playerBanned` | Once for each ban made while the server runs, whether or not it drops anyone: `author` (`by`), `reason`, `actionId` (the ban's code), `expiration` (Unix seconds, `false` for a permanent ban), `durationInput` (`permanent`, or the length in txAdmin's units, as `2 hours`, `1 day`, `2 weeks` or `1 month`; in minutes when none of them holds it whole), `durationTranslated` (the same words, nil for a permanent ban: Observer translates nothing), `targetNetId` (the connected player the ban drops, nil for none; of several, the one the ban names, else the lowest id), `targetIds` (`kind:value`, the licence first), `targetHwids` (hardware tokens), `targetName` (`identifiers` for a ban that names no player), `kickMessage` (the refusal message, as a player the ban drops reads it). The console's line holds only so much: tokens are left out first, then identifiers, from the last |
| none: a ban lifted | `txAdmin:events:actionRevoked` | Once for each ban lifted while the server runs: `actionId` (the ban's code), `actionType` (always `ban`), `actionReason` (the ban's reason), `actionAuthor` (who banned), `playerName` (`false` for a ban that names no player), `playerIds` (`kind:value`, the licence first), `playerHwids` (hardware tokens), `revokedBy` (who lifted the ban). The console's line holds only so much: tokens are left out first, then identifiers, from the last |

txAdmin showed announcements and direct messages to players itself. Observer does not: a resource
written for txAdmin's events shows them, the events alone show nothing.

## Not in version 6

Exports to read the next restart, the server's state or whether maintenance is on come later.

## Example

[`observer_example`](observer_example) finds the bridge and checks the contract version when it
starts, prints each event to the server console with its fields, and marks with short comments where
a team plugs in its notifications, its cancellation notice, its saves and what shows the staff's
announcements and messages. Its console command
`observer_example_label` gives a player a label, or clears it, to try the panel's side. It shows
nothing to players. Copy what fits into your own resources rather than running it as is.
