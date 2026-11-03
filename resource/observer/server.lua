-- Observer's side of the server: it reports health to Observer through FXServer's structured trace
-- channel, and turns Observer's commands into server events for other resources to handle.
-- It never shows anything to players.

-- Bumped when a field is renamed or removed, when resources must find the bridge another way, and
-- when the bridge gains an export, so that a resource can tell whether it is there; new fields keep
-- the version.
local CONTRACT_VERSION = 3
local HEARTBEAT_MS = 3000
local RESOURCES_SETTLE_MS = 500
local LABELS_SETTLE_MS = 1000
local PLAYERS_PAGE = 100
-- In characters, after the spaces around them.
local LABEL_NAME_MAX = 64
local LABEL_ID_MAX = 32
-- Starts and stops told in one report, past which they are only counted.
local RESOURCE_EVENTS_MAX = 64

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
local cfgRan = false

-- The thread first runs once the cfg has run, so `ready` means its resources were started.
CreateThread(function()
    cfgRan = true
    report('ready', { contractVersion = CONTRACT_VERSION })
    report('resources', { started = startedResources() })
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
    if not command or not EVENTS[command.event] or type(command.payload) ~= 'table' then
        return
    end
    TriggerEvent('observer:' .. command.event, command.payload)
    local txadmin = TXADMIN[command.event]
    if txadmin and GetConvarInt('observer_txadmin_events', 0) == 1 then
        for _, event in ipairs(txadmin(command.payload)) do
            TriggerEvent(event[1], event[2])
        end
    end
end, true)
