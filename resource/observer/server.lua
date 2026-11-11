-- Observer's side of the server: it reports health to Observer through FXServer's structured trace
-- channel, and turns Observer's commands into server events for other resources to handle.
-- It never shows anything to players but what the owner wrote: the refusal messages, one for bans,
-- one for maintenance mode, one for the allowlist, the kick message, and the name the server takes
-- in the server list during maintenance.

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
    announcement = true,
    directMessage = true,
    maintenance = true,
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

-- The server's vehicles, peds and objects, read from OneSync's entities while someone looks at
-- them. A sweep hands the frame back as it goes and stops past its budget, and the next ones space
-- out when one costs too much. Every entity native raises on a handle that names no entity any
-- more (FiveM's ServerGameState_Scripting.cpp): each is called under pcall, and a sweep that fails
-- never ends the thread.
local ENTITIES_EVERY_MIN_MS = 5000
local ENTITIES_EVERY_MAX_MS = 60000
local ENTITIES_FOR_MAX_MS = 60000
-- In microseconds: what a sweep works in one frame before it yields, and in all before it stops.
local ENTITIES_SLICE_US = 1000
local ENTITIES_BUDGET_US = 50000
-- Past either, in all or in the listing natives alone, the next sweeps come twice as far apart.
local ENTITIES_COSTLY_US = 25000
local ENTITIES_COSTLY_LIST_US = 10000
local ENTITIES_ROWS_MAX = 20000
local ENTITIES_PART_ROWS = 500
local ENTITIES_PART_BYTES = 65536
-- GetEntityScript walks every resource: this many lookups a sweep at most, once per entity.
local ENTITIES_LOOKUPS_MAX = 300
local ENTITIES_RESOURCES_MAX = 2048
local ENTITIES_RESOURCE_NAME_MAX = 64
local ENTITIES_DELETE_MAX = 100
-- DeleteEntity only queues the removal, on the server's sync thread (ServerGameState.h): what is
-- still there this much later was not deleted.
local ENTITIES_DELETE_CHECK_MS = 250
-- The numbers GetEntityType gives.
local ENTITY_PED = 1
local ENTITY_VEHICLE = 2
local ENTITY_OBJECT = 3
-- By what GetVehicleType answers; any other vehicle is 9.
local VEHICLE_CLASSES = {
    automobile = 1,
    bike = 2,
    boat = 3,
    heli = 4,
    plane = 5,
    submarine = 6,
    trailer = 7,
    train = 8,
}
local VEHICLE_OTHER = 9
local VEHICLE_TRAIN = 8
local ENTITY_OCCUPIED = 1
local ENTITY_RESOURCE_PENDING = 2

-- Which start of the bridge listed an entity: a bridge started again numbers them from 1 again.
local ENTITIES_EPOCH = GetGameTimer()
local entitiesEvery = 15000
local entitiesUntil = 0
local entitiesFollowing = false
-- How many times the cadence asked for the sweeps come apart, after costly ones.
local entitiesSpacing = 1
local entitiesSeq = 0
local entitiesSweep = 0
-- By net id, what never changes of an entity, read when it is first seen, and the serial drawn
-- for it then: net ids are used again, under another handle (Pool.h numbers each use of a slot).
local entities = {}
local entitySerial = 0

-- A clock in microseconds, which the game timer cannot give.
local entitiesClock = function()
    return GetGameTimer() * 1000
end
if type(os.nanotime) == 'function' then
    local ok, value = pcall(os.nanotime)
    if ok and type(value) == 'number' then
        entitiesClock = function()
            return os.nanotime() / 1000
        end
    end
end

local function finite(value)
    return type(value) == 'number' and value == value and value ~= math.huge and value ~= -math.huge
end

-- A model hash as an unsigned number: the natives give it signed.
local function unsignedHash(value)
    if type(value) ~= 'number' then
        return nil
    end
    value = math.floor(value)
    if value < 0 then
        value = value + 4294967296
    end
    if value < 0 or value > 4294967295 then
        return nil
    end
    return value
end

-- The players' own peds, never listed nor deleted, and the vehicles a player sits in.
local function playerEntities()
    local peds, occupied = {}, {}
    local ok, players = pcall(GetPlayers)
    if not ok or type(players) ~= 'table' then
        return peds, occupied
    end
    for index = 1, #players do
        local okPed, ped = pcall(GetPlayerPed, players[index])
        if okPed and type(ped) == 'number' and ped ~= 0 then
            peds[ped] = true
            local okVehicle, vehicle = pcall(GetVehiclePedIsIn, ped, false)
            if okVehicle and type(vehicle) == 'number' and vehicle ~= 0 then
                occupied[vehicle] = true
            end
        end
    end
    return peds, occupied
end

local function entityFirstSeen(netId, handle, kind)
    local model = unsignedHash(GetEntityModel(handle))
    if not model then
        return nil
    end
    local class = 0
    if kind == ENTITY_VEHICLE then
        class = VEHICLE_CLASSES[GetVehicleType(handle)] or VEHICLE_OTHER
    end
    local population = GetEntityPopulationType(handle)
    if type(population) ~= 'number' or population < 0 or population > 10 then
        population = 0
    end
    entitySerial = entitySerial + 1
    local entity = {
        handle = handle,
        serial = entitySerial,
        kind = kind,
        class = class,
        model = model,
        population = math.floor(population),
        -- A name, false for none, nil until it is looked up.
        resource = nil,
        seen = 0,
    }
    entities[netId] = entity
    return entity
end

-- One entity as `netId,serial,kind,class,model,x,y,z,owner,bucket,population,resource,flags`, the
-- resource as its place in the sweep's list of names, 0 for none.
local function entityRow(handle, kind, sweep)
    local netId = NetworkGetNetworkIdFromEntity(handle)
    if type(netId) ~= 'number' or netId <= 0 or netId > 65535 then
        return nil
    end
    local entity = entities[netId]
    if not entity or entity.handle ~= handle or entity.kind ~= kind then
        entity = entityFirstSeen(netId, handle, kind)
        if not entity then
            return nil
        end
    end
    entity.seen = entitiesSweep

    local coords = GetEntityCoords(handle)
    local x, y, z = coords.x, coords.y, coords.z
    if not finite(x) or not finite(y) or not finite(z) then
        return nil
    end
    local owner = NetworkGetEntityOwner(handle)
    if type(owner) ~= 'number' or owner < 0 then
        owner = -1
    end
    local bucket = GetEntityRoutingBucket(handle)
    if type(bucket) ~= 'number' then
        bucket = 0
    end
    local flags = 0
    if kind == ENTITY_VEHICLE and sweep.occupied[handle] then
        flags = flags + ENTITY_OCCUPIED
    end

    -- The resource that created it, as the server knew it the first time the entity was seen.
    if entity.resource == nil and sweep.lookups < ENTITIES_LOOKUPS_MAX then
        sweep.lookups = sweep.lookups + 1
        local ok, name = pcall(GetEntityScript, handle)
        if ok and type(name) == 'string' and #name > 0 and #name <= ENTITIES_RESOURCE_NAME_MAX then
            entity.resource = name
        else
            entity.resource = false
        end
    end
    local resource = 0
    if entity.resource == nil then
        flags = flags + ENTITY_RESOURCE_PENDING
    elseif entity.resource then
        resource = sweep.places[entity.resource]
        if not resource then
            if #sweep.names < ENTITIES_RESOURCES_MAX then
                sweep.names[#sweep.names + 1] = entity.resource
                resource = #sweep.names
                sweep.places[entity.resource] = resource
            else
                resource = 0
            end
        end
    end

    return string.format(
        '%d,%d,%d,%d,%d,%.1f,%.1f,%.1f,%d,%d,%d,%d,%d',
        netId,
        entity.serial,
        kind,
        entity.class,
        entity.model,
        x,
        y,
        z,
        math.floor(owner),
        math.floor(bucket),
        entity.population,
        resource,
        flags
    )
end

-- Why the server has no entity to list: only GTA V's are read, and only OneSync knows them.
local function entitiesUnavailable()
    if GetConvar('gamename', 'gta5') ~= 'gta5' then
        return 'game'
    end
    if GetConvar('onesync', 'off') == 'off' or type(GetAllVehicles) ~= 'function' then
        return 'onesync'
    end
end

local function entitiesCadence()
    return math.min(ENTITIES_EVERY_MAX_MS, entitiesEvery * entitiesSpacing)
end

local function sweepEntities()
    entitiesSeq = entitiesSeq % 2147483647 + 1
    local seq = entitiesSeq
    local unavailable = entitiesUnavailable()
    if unavailable then
        report('entities', {
            epoch = ENTITIES_EPOCH,
            seq = seq,
            -- Never alone on its line: the lines ending in `= true,` are the events of the contract.
            last = true, part = 1,
            rows = '',
            unavailable = unavailable,
        })
        return
    end
    entitiesSweep = entitiesSweep + 1
    local worked, frames = 0, 1
    local slice = entitiesClock()
    local function yield()
        worked = worked + (entitiesClock() - slice)
        frames = frames + 1
        Wait(0)
        slice = entitiesClock()
    end
    local function spent()
        return worked + (entitiesClock() - slice)
    end

    local playerPeds, occupied = playerEntities()
    local players = 0
    for _ in pairs(playerPeds) do
        players = players + 1
    end

    -- Each of the three takes the lock on the server's entities and makes the handles it lacks:
    -- one a frame.
    local lists, counts, listing = {}, {}, 0
    for _, source in ipairs({
        { 'vehicles', GetAllVehicles, ENTITY_VEHICLE },
        { 'objects', GetAllObjects, ENTITY_OBJECT },
        { 'peds', GetAllPeds, ENTITY_PED },
    }) do
        local started = entitiesClock()
        local ok, list = pcall(source[2])
        listing = listing + (entitiesClock() - started)
        if ok and type(list) == 'table' then
            counts[source[1]] = #list
            lists[#lists + 1] = { source[3], list }
        end
        yield()
    end

    local sweep = { occupied = occupied, lookups = 0, places = {}, names = {} }
    local rows, part, bytes, listed, failed = {}, 0, 0, 0, 0
    local truncated = false
    for _, each in ipairs(lists) do
        local kind, list = each[1], each[2]
        for index = 1, #list do
            if listed >= ENTITIES_ROWS_MAX or spent() >= ENTITIES_BUDGET_US then
                truncated = true
                break
            end
            local handle = list[index]
            if not (kind == ENTITY_PED and playerPeds[handle]) then
                local ok, row = pcall(entityRow, handle, kind, sweep)
                if not ok then
                    failed = failed + 1
                elseif row then
                    rows[#rows + 1] = row
                    listed = listed + 1
                    bytes = bytes + #row + 1
                    if #rows >= ENTITIES_PART_ROWS or bytes >= ENTITIES_PART_BYTES then
                        part = part + 1
                        report('entities', {
                            epoch = ENTITIES_EPOCH,
                            seq = seq,
                            part = part,
                            last = false,
                            rows = table.concat(rows, ';'),
                        })
                        rows, bytes = {}, 0
                    end
                end
            end
            if entitiesClock() - slice >= ENTITIES_SLICE_US then
                yield()
            end
        end
        if truncated then
            break
        end
    end

    -- A sweep cut short did not see every entity: those it missed keep their serial.
    if not truncated then
        for netId, entity in pairs(entities) do
            if entity.seen ~= entitiesSweep then
                entities[netId] = nil
            end
        end
    end
    worked = spent()

    if worked > ENTITIES_COSTLY_US or listing > ENTITIES_COSTLY_LIST_US then
        entitiesSpacing = math.min(entitiesSpacing * 2, math.ceil(ENTITIES_EVERY_MAX_MS / entitiesEvery))
    elseif entitiesSpacing > 1 and worked < ENTITIES_COSTLY_US / 2 and listing < ENTITIES_COSTLY_LIST_US / 2 then
        entitiesSpacing = math.max(1, entitiesSpacing // 2)
    end

    counts.playerPeds = players
    report('entities', {
        epoch = ENTITIES_EPOCH,
        seq = seq,
        last = true, part = part + 1,
        rows = table.concat(rows, ';'),
        counts = counts,
        listed = listed,
        failed = failed,
        truncated = truncated,
        workUs = math.floor(worked),
        listUs = math.floor(listing),
        frames = frames,
        everyMs = entitiesCadence(),
        spaced = entitiesSpacing > 1,
        resources = sweep.names,
    })
end

-- One thread at most, which ends once Observer stopped asking.
local function followEntities()
    if entitiesFollowing then
        return
    end
    entitiesFollowing = true
    CreateThread(function()
        while GetGameTimer() < entitiesUntil do
            local started = GetGameTimer()
            pcall(sweepEntities)
            -- In steps, so that the thread ends within a second of Observer's last ask running out.
            local nextAt = started + entitiesCadence()
            while GetGameTimer() < entitiesUntil and GetGameTimer() < nextAt do
                Wait(math.max(0, math.min(1000, nextAt - GetGameTimer())))
            end
        end
        entitiesSpacing = 1
        entitiesFollowing = false
    end)
end

local function listEntities(payload)
    if type(payload.everyMs) ~= 'number' or type(payload.forMs) ~= 'number' then
        return
    end
    entitiesEvery = math.max(ENTITIES_EVERY_MIN_MS, math.min(ENTITIES_EVERY_MAX_MS, math.floor(payload.everyMs)))
    entitiesUntil = GetGameTimer() + math.max(0, math.min(ENTITIES_FOR_MAX_MS, math.floor(payload.forMs)))
    followEntities()
end

-- What becomes of one entity Observer asks to delete, and its handle once the engine was asked.
-- Never a player's ped, a vehicle a player sits in or a train, and only the entity the last sweep
-- listed under this net id.
local function deleteEntity(netId, serial, playerPeds, occupied)
    local okHandle, handle = pcall(NetworkGetEntityFromNetworkId, netId)
    if not okHandle or type(handle) ~= 'number' or handle == 0 then
        return 'gone'
    end
    local entity = entities[netId]
    if not entity or entity.serial ~= serial or entity.handle ~= handle then
        return 'changed'
    end
    local okModel, model = pcall(GetEntityModel, handle)
    if not okModel then
        return 'gone'
    end
    if unsignedHash(model) ~= entity.model then
        return 'changed'
    end
    if entity.kind == ENTITY_PED then
        if playerPeds[handle] then
            return 'player'
        end
        local ok, player = pcall(IsPedAPlayer, handle)
        if not ok then
            return 'error'
        end
        if player then
            return 'player'
        end
    elseif entity.kind == ENTITY_VEHICLE then
        if occupied[handle] then
            return 'occupied'
        end
        local ok, vehicleType = pcall(GetVehicleType, handle)
        if not ok then
            return 'error'
        end
        if vehicleType == 'train' or entity.class == VEHICLE_TRAIN then
            return 'train'
        end
    end
    if not pcall(DeleteEntity, handle) then
        return 'error'
    end
    return 'deleted', handle
end

local function deleteListed(payload)
    local request, targets = payload.request, payload.targets
    if type(request) ~= 'number' or type(payload.deadline) ~= 'number' or type(targets) ~= 'table' then
        return
    end
    -- Net ids and serials, one after the other.
    local count = #targets // 2
    if count == 0 or count > ENTITIES_DELETE_MAX or #targets % 2 ~= 0 then
        return
    end
    for index = 1, #targets do
        if type(targets[index]) ~= 'number' then
            return
        end
    end
    CreateThread(function()
        local outcomes, deleted = {}, {}
        local playerPeds, occupied = playerEntities()
        local slice = entitiesClock()
        for index = 1, count do
            local netId, serial = math.floor(targets[index * 2 - 1]), math.floor(targets[index * 2])
            local outcome
            if payload.epoch ~= ENTITIES_EPOCH then
                outcome = 'changed'
            elseif os.time() > payload.deadline then
                -- A server that hitched ran the command late: what is not done yet is not done.
                outcome = 'late'
            else
                local ok, became, handle = pcall(deleteEntity, netId, serial, playerPeds, occupied)
                outcome = ok and became or 'error'
                if ok and handle then
                    deleted[#deleted + 1] = { index, handle, netId }
                end
            end
            outcomes[index] = { netId, outcome }
            if entitiesClock() - slice >= ENTITIES_SLICE_US then
                Wait(0)
                slice = entitiesClock()
            end
        end
        if #deleted > 0 then
            Wait(ENTITIES_DELETE_CHECK_MS)
            for _, each in ipairs(deleted) do
                local ok, exists = pcall(DoesEntityExist, each[2])
                if ok and exists then
                    outcomes[each[1]][2] = 'unconfirmed'
                else
                    entities[each[3]] = nil
                end
            end
        end
        report('entitiesDeleted', { request = request, outcomes = outcomes })
    end)
end

-- Whether the owner asked for txAdmin's events beside Observer's own.
local function txadminEvents()
    return GetConvarInt('observer_txadmin_events', 0) == 1
end

-- One of txAdmin's events, raised with the payload Observer wrote for it when the owner asked.
local function txadminEvent(name, payload)
    if type(payload.event) == 'table' and txadminEvents() then
        TriggerEvent(name, payload.event)
    end
end

-- `observer:playerKicked` for one player of a kick. Handlers may keep the table: each gets its own.
local function kickedEvent(event, id)
    local copy = {}
    for key, value in pairs(event) do
        copy[key] = value
    end
    copy.player = id
    return copy
end

-- The convars FiveM's server list reads a server's name from, each with the server convar that
-- keeps what it held while the server carries its maintenance name: convars, so that a bridge
-- restarted in the middle of a run still gives the names back.
local NAME_KEPT = 'observer_name_kept'
local NAMES = {
    { convar = 'sv_hostname', kept = 'observer_name_hostname' },
    { convar = 'sv_projectName', kept = 'observer_name_project', info = true },
}

-- `sv_projectName` is a server info variable, set as the cfg sets it, with `sets`.
local function setName(name, value)
    if name.info then
        SetConvarServerInfo(name.convar, value)
    else
        SetConvar(name.convar, value)
    end
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
    -- Every event of a kick is raised before its first drop: all its players are still connected.
    kick = function(payload)
        local event = payload.event
        if type(payload.message) ~= 'string' or payload.message == '' or type(event) ~= 'table' then
            return
        end
        local everyone = payload.all == true
        local players = everyone and GetPlayers() or payload.players
        if type(players) ~= 'table' then
            return
        end
        local txadmin = txadminEvents()
        local function txadminKicked(target)
            TriggerEvent('txAdmin:events:playerKicked', {
                target = target,
                author = event.by,
                reason = event.reason,
                dropMessage = payload.message,
            })
        end
        local kicked = {}
        for _, player in ipairs(players) do
            local id = connected(player)
            if id then
                kicked[#kicked + 1] = id
                TriggerEvent('observer:playerKicked', kickedEvent(event, id))
                if txadmin and not everyone then
                    txadminKicked(id)
                end
            end
        end
        -- txAdmin tells of a kick of everyone once, with -1 for the player.
        if txadmin and everyone then
            txadminKicked(-1)
        end
        for _, id in ipairs(kicked) do
            DropPlayer(tostring(id), payload.message)
        end
    end,
    -- A ban just made, for resources written against txAdmin: Observer raises nothing of its own
    -- for a ban that drops nobody.
    banned = function(payload)
        txadminEvent('txAdmin:events:playerBanned', payload)
    end,
    -- A ban just lifted, for resources written against txAdmin: Observer raises nothing of its own.
    lifted = function(payload)
        txadminEvent('txAdmin:events:actionRevoked', payload)
    end,
    positions = function(payload)
        if type(payload.everyMs) ~= 'number' or type(payload.forMs) ~= 'number' then
            return
        end
        positionsEvery = math.max(POSITIONS_EVERY_MIN_MS, math.min(POSITIONS_EVERY_MAX_MS, math.floor(payload.everyMs)))
        positionsUntil = GetGameTimer() + math.max(0, math.min(POSITIONS_FOR_MAX_MS, math.floor(payload.forMs)))
        followPositions()
    end,
    entities = function(payload)
        listEntities(payload)
    end,
    deleteEntities = function(payload)
        deleteListed(payload)
    end,
    serverName = function(payload)
        local kept = GetConvar(NAME_KEPT, '')
        if type(payload.name) == 'string' and payload.name ~= '' then
            if kept == '' then
                -- A server without a project name is listed by its host name alone: it gets none.
                local renamed = {}
                for _, name in ipairs(NAMES) do
                    local held = GetConvar(name.convar, '')
                    if held ~= '' or name.convar == 'sv_hostname' then
                        SetConvar(name.kept, held)
                        renamed[#renamed + 1] = name.convar
                    end
                end
                kept = table.concat(renamed, ' ')
                SetConvar(NAME_KEPT, kept)
            end
            for _, name in ipairs(NAMES) do
                if kept:find(name.convar, 1, true) then
                    setName(name, payload.name)
                end
            end
        elseif kept ~= '' then
            for _, name in ipairs(NAMES) do
                if kept:find(name.convar, 1, true) then
                    setName(name, GetConvar(name.kept, ''))
                end
            end
            SetConvar(NAME_KEPT, '')
        end
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

-- Every resource FXServer knows, for Observer alone: no part of the contract with the server's
-- resources. In pages, as a server holds hundreds.
local RESOURCE_LIST_PAGE = 100
-- A table given to `dependencies` in a manifest is kept under the key less its last letter, which
-- FXServer reads its dependencies from as well.
local NEEDS_KEYS = { 'dependency', 'dependencie' }
local PROVIDES_KEYS = { 'provide' }
local resourceLists = 0

local function manifestEntries(name, keys)
    local entries = {}
    for _, key in ipairs(keys) do
        for index = 0, GetNumResourceMetadata(name, key) - 1 do
            entries[#entries + 1] = GetResourceMetadata(name, key, index)
        end
    end
    return #entries > 0 and entries or nil
end

local function reportResourceList()
    resourceLists = resourceLists + 1
    local list = resourceLists
    local names = {}
    for index = 0, GetNumResources() - 1 do
        local name = GetResourceByFindIndex(index)
        if name then
            names[#names + 1] = name
        end
    end
    table.sort(names)
    local pages = math.max(1, math.ceil(#names / RESOURCE_LIST_PAGE))
    for page = 1, pages do
        local entries = {}
        for index = (page - 1) * RESOURCE_LIST_PAGE + 1, math.min(page * RESOURCE_LIST_PAGE, #names) do
            local name = names[index]
            entries[#entries + 1] = {
                name = name,
                state = GetResourceState(name),
                path = GetResourcePath(name),
                version = GetResourceMetadata(name, 'version', 0),
                needs = manifestEntries(name, NEEDS_KEYS),
                provides = manifestEntries(name, PROVIDES_KEYS),
            }
        end
        report('resource_list', { list = list, page = page, last = page == pages, resources = entries })
        if page < pages then
            Wait(0)
        end
    end
end

-- `refresh` reads every manifest again and may find new resources.
AddEventHandler('onResourceListRefresh', function()
    CreateThread(reportResourceList)
end)

-- The thread first runs once the cfg has run, so `ready` means its resources were started.
CreateThread(function()
    cfgRan = true
    report('ready', { contractVersion = CONTRACT_VERSION })
    report('resources', { started = startedResources() })
    -- Always, even with none: a bridge started again holds nothing of what the one before held.
    reportHolds()
    reportResourceList()
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
    announcement = function(payload)
        return {
            { 'txAdmin:events:announcement', { message = payload.message, author = payload.by } },
        }
    end,
    directMessage = function(payload)
        return {
            {
                'txAdmin:events:playerDirectMessage',
                { target = payload.player, author = payload.by, message = payload.message },
            },
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
    if txadmin and txadminEvents() then
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
