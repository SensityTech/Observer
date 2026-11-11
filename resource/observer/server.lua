-- Observer's side of the server: it reports health to Observer through FXServer's structured trace
-- channel, and turns Observer's commands into server events for other resources to handle.
-- It never shows anything to players but one text: the refusal message the owner wrote for bans.

-- Bumped when a field is renamed or removed, when resources must find the bridge another way, and
-- when the bridge gains an export, so that a resource can tell whether it is there; new fields keep
-- the version.
local CONTRACT_VERSION = 7
local HEARTBEAT_MS = 3000
local RESOURCES_SETTLE_MS = 500
local LABELS_SETTLE_MS = 1000
local PLAYERS_PAGE = 100
-- In characters, after the spaces around them.
local LABEL_NAME_MAX = 64
local LABEL_ID_MAX = 32
-- Starts and stops told in one report, past which they are only counted.
local RESOURCE_EVENTS_MAX = 64
-- How long a connecting player waits for Observer's verdict before getting in anyway: Observer
-- checks them again once they joined.
local VERDICT_WAIT_MS = 5000
local VERDICT_POLL_MS = 50
-- While someone watches the live map: how often positions go out, at most, and how long one ask
-- of Observer lasts before it must ask again.
local POSITIONS_EVERY_MIN_MS = 1000
local POSITIONS_EVERY_MAX_MS = 10000
local POSITIONS_FOR_MAX_MS = 60000
-- The most resources that can hold a stop at once.
local HOLDS_MAX = 64
local HOLDS_SETTLE_MS = 500

-- The events Observer may raise, and nothing else.
local EVENTS = {
    restartScheduled = true,
    restartSkipped = true,
    restartRescheduled = true,
    shuttingDown = true,
    startChanges = true,
}

local function report(kind, fields)
    local message = fields or {}
    message.type = 'observer'
    message.kind = kind
    PrintStructuredTrace(json.encode(message))
end

local function startedResources()
    local started = {}
    for index = 0, GetNumResources() - 1 do
        local name = GetResourceByFindIndex(index)
        if name and GetResourceState(name) == 'started' then
            started[#started + 1] = name
        end
    end
    table.sort(started)
    return started
end

exports('getContractVersion', function()
    return CONTRACT_VERSION
end)

-- What a player shows, as FXServer gives it: Observer checks, cleans and keeps what it needs.
local function shown(id)
    return {
        identifiers = GetPlayerIdentifiers(id),
        tokens = GetPlayerTokens(id),
        name = GetPlayerName(id),
    }
end

-- The players already connected when the bridge starts, after a restart of the bridge for one, in
-- pages: one report for a full server would pass the size Observer reads.
local function reportPlayers()
    local players = GetPlayers()
    local t = GetGameTimer()
    local pages = math.max(1, math.ceil(#players / PLAYERS_PAGE))
    for page = 1, pages do
        local entries = {}
        for index = (page - 1) * PLAYERS_PAGE + 1, math.min(page * PLAYERS_PAGE, #players) do
            local id = tonumber(players[index])
            -- A player who left while the pages were sent has no name any more.
            if id and type(GetPlayerName(id)) == 'string' then
                local entry = shown(id)
                entry.id = id
                entries[#entries + 1] = entry
            end
        end
        report('players', { t = t, page = page, last = page == pages, players = entries })
        if page < pages then
            Wait(0)
        end
    end
end

AddEventHandler('playerJoining', function()
    local id = tonumber(source)
    -- A resource may drop the player inside its own handler, before this one runs.
    if not id or id <= 0 or type(GetPlayerName(id)) ~= 'string' then
        return
    end
    local fields = shown(id)
    fields.id = id
    fields.t = GetGameTimer()
    report('join', fields)
end)

-- The labels given or cleared since the last report, by player: the latest of each, with the
-- resource that gave it.
local labels = {}
local labelsPending = false

local function reportLabels()
    labelsPending = false
    local entries = {}
    for id, change in pairs(labels) do
        entries[#entries + 1] = { id = id, label = change.label or nil, resource = change.resource }
    end
    labels = {}
    for first = 1, #entries, PLAYERS_PAGE do
        report('labels', { labels = table.move(entries, first, math.min(first + PLAYERS_PAGE - 1, #entries), 1, {}) })
    end
end

-- A resource that sets labels in a loop sends one report a second, never one per call.
local function labelChanged(id, label, resource)
    labels[id] = { label = label, resource = resource }
    if not labelsPending then
        labelsPending = true
        SetTimeout(LABELS_SETTLE_MS, reportLabels)
    end
end

-- A label's text: nil when absent or blank, or nil and why it is refused.
local function labelText(value, max)
    if value == nil then
        return nil
    end
    if type(value) ~= 'string' then
        return nil, 'invalid'
    end
    value = value:match('^%s*(.*%S)')
    if not value then
        return nil
    end
    local length = utf8.len(value)
    if not length then
        return nil, 'invalid'
    end
    if length > max then
        return nil, 'tooLong'
    end
    return value
end

local function connected(player)
    local id = tonumber(player)
    if id and id > 0 and type(GetPlayerName(id)) == 'string' then
        return id
    end
end

-- What the panel shows for a player beside their FiveM name, as the server's resources call them.
-- Display data: Observer checks it again and never takes it for the player's identity.
exports('setPlayerLabel', function(player, label)
    local id = connected(player)
    if not id then
        return false, 'notConnected'
    end
    if type(label) ~= 'table' then
        return false, 'invalid'
    end
    local name, nameRefused = labelText(label.name, LABEL_NAME_MAX)
    local labelId, idRefused = labelText(label.id, LABEL_ID_MAX)
    local refused = nameRefused or idRefused
    if refused then
        return false, refused
    end
    if not name and not labelId then
        return false, 'invalid'
    end
    labelChanged(id, { name = name, id = labelId }, GetInvokingResource())
    return true
end)

exports('clearPlayerLabel', function(player)
    local id = connected(player)
    if not id then
        return false, 'notConnected'
    end
    labelChanged(id, false, GetInvokingResource())
    return true
end)

local cfgRan = false

-- The resources that hold the stop: Observer has FXServer quit once each has released it or
-- stopped, or once the owner's limit has passed.
local holds = {}
local holdsCount = 0
-- Holders that stopped during the stop, which then release nothing: told once.
local holdsStopped = {}
local holdsPending = false
-- From the first observer:shuttingDown until FXServer quits.
local stopping = false

local function reportHolds()
    holdsPending = false
    local holding = {}
    for resource in pairs(holds) do
        holding[#holding + 1] = resource
    end
    table.sort(holding)
    local stopped = holdsStopped
    holdsStopped = {}
    report('holds', {
        stopping = stopping,
        holding = #holding > 0 and holding or nil,
        stopped = #stopped > 0 and stopped or nil,
    })
end

-- During a stop each release goes out at once: Observer waits on it. Before one, the resources
-- that ask as the server starts are told together, after `ready`.
local function holdsChanged()
    if not cfgRan then
        return
    end
    if stopping then
        reportHolds()
        return
    end
    if not holdsPending then
        holdsPending = true
        SetTimeout(HOLDS_SETTLE_MS, reportHolds)
    end
end

-- Every stop of the server waits for the calling resource from now on, until it releases the stop
-- or stops itself. Observer waits no longer than the owner's limit, and names who held past it.
exports('holdShutdown', function()
    local resource = GetInvokingResource()
    if type(resource) ~= 'string' or resource == '' then
        return false, 'invalid'
    end
    if holds[resource] then
        return true
    end
    if holdsCount >= HOLDS_MAX then
        return false, 'tooMany'
    end
    holds[resource] = true
    holdsCount = holdsCount + 1
    holdsChanged()
    return true
end)

-- The calling resource has saved what it had to: the stop under way no longer waits for it.
exports('releaseShutdown', function()
    if not stopping then
        return false, 'notStopping'
    end
    local resource = GetInvokingResource()
    if type(resource) ~= 'string' or not holds[resource] then
        return false, 'notHeld'
    end
    holds[resource] = nil
    holdsCount = holdsCount - 1
    holdsChanged()
    return true
end)

-- The connections waiting for Observer's verdict, by request: false until it comes.
local verdicts = {}
local lastRequest = 0

AddEventHandler('playerConnecting', function(name, setKickReason, deferrals)
    local player = source
    deferrals.defer()
    Wait(0)
    lastRequest = lastRequest % 2147483647 + 1
    local request = lastRequest
    local fields = shown(player)
    fields.name = fields.name or name
    fields.request = request
    verdicts[request] = false
    report('connecting', fields)
    local waited = 0
    while verdicts[request] == false and waited < VERDICT_WAIT_MS do
        Wait(VERDICT_POLL_MS)
        waited = waited + VERDICT_POLL_MS
    end
    local verdict = verdicts[request]
    verdicts[request] = nil
    local refusal = verdict and verdict.refusal
    if type(refusal) == 'table' and type(refusal.message) == 'string' and refusal.message ~= '' then
        deferrals.done(refusal.message)
        if type(refusal.event) == 'table' then
            TriggerEvent('observer:playerRefused', refusal.event)
        end
        return
    end
    deferrals.done()
end)

-- Where the players stand, read from OneSync's entities: without it the server has no position.
local positionsEvery = 2000
local positionsUntil = 0
local positionsFollowing = false

local function tenth(value)
    return math.floor(value * 10 + 0.5) / 10
end

local function reportPositions()
    local t = GetGameTimer()
    if GetConvar('onesync', 'off') == 'off' then
        report('positions', { t = t, onesync = false, page = 1, last = true, players = {} })
        return
    end
    local players = GetPlayers()
    local pages = math.max(1, math.ceil(#players / PLAYERS_PAGE))
    for page = 1, pages do
        local entries = {}
        for index = (page - 1) * PLAYERS_PAGE + 1, math.min(page * PLAYERS_PAGE, #players) do
            local id = tonumber(players[index])
            local ped = id and GetPlayerPed(id) or 0
            -- No ped yet while the player loads.
            if ped ~= 0 then
                local coords = GetEntityCoords(ped)
                entries[#entries + 1] = {
                    id = id,
                    x = tenth(coords.x),
                    y = tenth(coords.y),
                    z = tenth(coords.z),
                    h = tenth(GetEntityHeading(ped)),
                }
            end
        end
        report('positions', { t = t, onesync = true, page = page, last = page == pages, players = entries })
    end
end

-- One thread at most, which ends once Observer stopped asking.
local function followPositions()
    if positionsFollowing then
        return
    end
    positionsFollowing = true
    CreateThread(function()
        while GetGameTimer() < positionsUntil do
            reportPositions()
            Wait(positionsEvery)
        end
        positionsFollowing = false
    end)
end

-- What Observer has the bridge do itself, and nothing else.
local ACTIONS = {
    verdict = function(payload)
        if verdicts[payload.request] == false then
            verdicts[payload.request] = payload
        end
    end,
    drop = function(payload)
        local id = connected(payload.player)
        if not id or type(payload.message) ~= 'string' or payload.message == '' then
            return
        end
        if type(payload.event) == 'table' then
            TriggerEvent('observer:playerBanned', payload.event)
        end
        DropPlayer(tostring(id), payload.message)
    end,
    positions = function(payload)
        if type(payload.everyMs) ~= 'number' or type(payload.forMs) ~= 'number' then
            return
        end
        positionsEvery = math.max(POSITIONS_EVERY_MIN_MS, math.min(POSITIONS_EVERY_MAX_MS, math.floor(payload.everyMs)))
        positionsUntil = GetGameTimer() + math.max(0, math.min(POSITIONS_FOR_MAX_MS, math.floor(payload.forMs)))
        followPositions()
    end,
}

AddEventHandler('playerDropped', function(reason, resource, category)
    local id = tonumber(source)
    if not id or id <= 0 then
        return
    end
    -- Its net id may be someone else's by the time the label would go out.
    labels[id] = nil
    local ped
    -- With OneSync a player has a ped once they spawned: without one, they were still loading.
    if GetConvar('onesync', 'off') ~= 'off' then
        ped = GetPlayerPed(id) ~= 0
    end
    report('drop', { id = id, t = GetGameTimer(), reason = reason, resource = resource, category = category, ped = ped })
end)

-- The thread first runs once the cfg has run, so `ready` means its resources were started.
CreateThread(function()
    cfgRan = true
    report('ready', { contractVersion = CONTRACT_VERSION })
    report('resources', { started = startedResources() })
    -- Always, even with none: a bridge started again holds nothing of what the one before held.
    reportHolds()
    reportPlayers()
    while true do
        report('hb')
        Wait(HEARTBEAT_MS)
    end
end)

local resourcesPending = false
-- What started and stopped since the last report, once the cfg has run: a restart leaves the list
-- of started resources as it was, so only these tell it.
local resourceEvents = {}
local resourceEventsDropped = 0
local function resourcesChanged(name, change)
    if cfgRan then
        if #resourceEvents < RESOURCE_EVENTS_MAX then
            resourceEvents[#resourceEvents + 1] = { name, change }
        else
            resourceEventsDropped = resourceEventsDropped + 1
        end
    end
    if resourcesPending then
        return
    end
    resourcesPending = true
    -- A restart stops and starts in a row; one report covers both.
    SetTimeout(RESOURCES_SETTLE_MS, function()
        resourcesPending = false
        local events, dropped = resourceEvents, resourceEventsDropped
        resourceEvents, resourceEventsDropped = {}, 0
        report('resources', {
            started = startedResources(),
            events = #events > 0 and events or nil,
            dropped = dropped > 0 and dropped or nil,
        })
    end)
end
AddEventHandler('onResourceStart', function(name)
    resourcesChanged(name, 'start')
end)
AddEventHandler('onResourceStop', function(name)
    resourcesChanged(name, 'stop')
    -- A stopped resource saves nothing more: no stop waits for it.
    if holds[name] then
        holds[name] = nil
        holdsCount = holdsCount - 1
        if stopping then
            holdsStopped[#holdsStopped + 1] = name
        end
        holdsChanged()
    end
end)

local function secondsUntil(at)
    if type(at) ~= 'number' then
        return 0
    end
    return math.max(0, at // 1000 - os.time())
end

-- txAdmin counts its warnings in whole minutes, and resources written for it divide by 60.
local function wholeMinutes(seconds)
    if type(seconds) ~= 'number' then
        return 0
    end
    return math.ceil(seconds / 60) * 60
end

-- The events txAdmin raised where Observer raises its own, with txAdmin's payloads, for resources
-- written against txAdmin. Each one lists them in the order txAdmin raised them.
local TXADMIN = {
    restartScheduled = function(payload)
        local secondsRemaining = wholeMinutes(payload.secondsRemaining)
        return {
            { 'txAdmin:events:scheduledRestart', { secondsRemaining = secondsRemaining, translatedMessage = '' } },
        }
    end,
    restartSkipped = function(payload)
        local secondsRemaining = secondsUntil(payload.at)
        local temporary = payload.reason == 'oneOff'
        return {
            {
                'txAdmin:events:scheduledRestartSkipped',
                { secondsRemaining = secondsRemaining, temporary = temporary, author = payload.by },
            },
            -- Deprecated by txAdmin, which still raises it.
            {
                'txAdmin:events:skippedNextScheduledRestart',
                { secondsRemaining = secondsRemaining, temporary = temporary },
            },
        }
    end,
    shuttingDown = function(payload)
        return {
            { 'txAdmin:events:serverShuttingDown', { delay = payload.delayMs, author = payload.by, message = '' } },
        }
    end,
}

-- Observer sends JSON in lowercase hex, which no quote, semicolon or line break can break up.
local function decode(hex)
    if type(hex) ~= 'string' or #hex == 0 or #hex % 2 ~= 0 or hex:find('[^0-9a-f]') then
        return nil
    end
    local text = hex:gsub('..', function(pair)
        return string.char(tonumber(pair, 16))
    end)
    local ok, command = pcall(json.decode, text)
    if ok and type(command) == 'table' then
        return command
    end
end

RegisterCommand('observer_cmd', function(source, args)
    -- Only from the server console, where only Observer writes, and never from another resource.
    if source ~= 0 or GetInvokingResource() ~= nil then
        return
    end
    local command = decode(args[1])
    if not command or type(command.payload) ~= 'table' then
        return
    end
    local action = ACTIONS[command.action]
    if action then
        action(command.payload)
        return
    end
    if not EVENTS[command.event] then
        return
    end
    local stop = command.event == 'shuttingDown'
    if stop then
        stopping = true
    end
    TriggerEvent('observer:' .. command.event, command.payload)
    local txadmin = TXADMIN[command.event]
    if txadmin and GetConvarInt('observer_txadmin_events', 0) == 1 then
        for _, event in ipairs(txadmin(command.payload)) do
            TriggerEvent(event[1], event[2])
        end
    end
    if stop then
        -- Who holds the stop once every handler has run up to its first wait: Observer counts on
        -- this report to know the event was raised.
        reportHolds()
    end
end, true)
