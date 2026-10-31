-- Observer's side of the server: it reports health to Observer through FXServer's structured trace
-- channel, and turns Observer's commands into server events for other resources to handle.
-- It never shows anything to players.

-- Bumped only when a field is renamed or removed; new fields keep the version.
local CONTRACT_VERSION = 1
local HEARTBEAT_MS = 3000
local RESOURCES_SETTLE_MS = 500
local PLAYERS_PAGE = 100

-- The events Observer may raise, and nothing else.
local EVENTS = {
    restartScheduled = true,
    restartSkipped = true,
    restartRescheduled = true,
    shuttingDown = true,
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

-- The players already connected when the bridge starts, after `restart observer` for one, in
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

AddEventHandler('playerDropped', function(reason, resource, category)
    local id = tonumber(source)
    if not id or id <= 0 then
        return
    end
    local ped
    -- With OneSync a player has a ped once they spawned: without one, they were still loading.
    if GetConvar('onesync', 'off') ~= 'off' then
        ped = GetPlayerPed(id) ~= 0
    end
    report('drop', { id = id, t = GetGameTimer(), reason = reason, resource = resource, category = category, ped = ped })
end)

-- The thread first runs once the cfg has run, so `ready` means its resources were started.
CreateThread(function()
    report('ready', { contractVersion = CONTRACT_VERSION })
    report('resources', { started = startedResources() })
    reportPlayers()
    while true do
        report('hb')
        Wait(HEARTBEAT_MS)
    end
end)

local resourcesPending = false
local function resourcesChanged()
    if resourcesPending then
        return
    end
    resourcesPending = true
    -- A restart stops and starts in a row; one report covers both.
    SetTimeout(RESOURCES_SETTLE_MS, function()
        resourcesPending = false
        report('resources', { started = startedResources() })
    end)
end
AddEventHandler('onResourceStart', resourcesChanged)
AddEventHandler('onResourceStop', resourcesChanged)

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
