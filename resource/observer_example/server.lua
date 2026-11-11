-- Listens to the server events of Observer's integration contract (resource/CONTRACT.md) and prints
-- each one with its fields, holds each stop until its saves are done, and gives players labels for
-- Observer's panel from the console. It shows nothing to players: the comments mark where your own
-- notifications, cancellation notices, saves and labels would go.

-- The contract this resource was written for.
local CONTRACT_VERSION = 7

local function log(text)
    print(('[observer_example] %s'):format(text))
end

-- Numbers come from JSON: print whole ones without a decimal point.
local function show(value)
    if math.type(value) == 'float' and value == math.floor(value) then
        return ('%.0f'):format(value)
    end
    return tostring(value)
end

local function fields(payload, names)
    local parts = {}
    for _, name in ipairs(names) do
        parts[#parts + 1] = ('%s=%s'):format(name, show(payload[name]))
    end
    return table.concat(parts, ' ')
end

CreateThread(function()
    -- The bridge's name changes with every start of FXServer: Observer gives it in this convar.
    local bridge = GetConvar('observer_bridge', '')
    if bridge == '' or GetResourceState(bridge) ~= 'started' then
        log('^3the observer bridge is not running: is this server started by Observer?^7')
        return
    end
    local version = exports[bridge]:getContractVersion()
    if version ~= CONTRACT_VERSION then
        log(('^3Observer speaks contract %s, this resource was written for %d^7'):format(
            show(version), CONTRACT_VERSION))
    end
    -- Once, as the resource starts: from now on every stop waits until this resource has released
    -- it, within the limit the owner set. Holding a stop came with contract 7.
    if type(version) == 'number' and version >= 7 then
        local ok, refused = exports[bridge]:holdShutdown()
        if not ok then
            log('the stop is not held: ' .. tostring(refused))
        end
    end
end)

-- Stands for your saves: it takes observer_example_save_ms milliseconds, 0 by default. Set the
-- convar to try a stop that waits, and past the owner's limit to see Observer quit without it.
local function save()
    Wait(GetConvarInt('observer_example_save_ms', 0))
end

-- AddEventHandler only: never RegisterNetEvent these names, or clients could raise them.

AddEventHandler('observer:restartScheduled', function(payload)
    log('restartScheduled ' .. fields(payload, { 'at', 'secondsRemaining', 'reason' }))
    -- Your notifications go here: the server restarts in payload.secondsRemaining seconds.
end)

AddEventHandler('observer:restartSkipped', function(payload)
    log('restartSkipped ' .. fields(payload, { 'at', 'by', 'reason' }))
    -- Your cancellation notice goes here: the restart announced for payload.at will not happen.
end)

AddEventHandler('observer:restartRescheduled', function(payload)
    log('restartRescheduled ' .. fields(payload, { 'at', 'previousAt', 'by', 'reason' }))
    -- Your cancellation notice goes here: the restart moved from payload.previousAt to payload.at.
end)

AddEventHandler('observer:shuttingDown', function(payload)
    log('shuttingDown ' .. fields(payload, { 'delayMs', 'maxDelayMs', 'reason', 'by' }))
    -- Your saves go here: FXServer quits once this resource has released the stop, and after
    -- payload.maxDelayMs milliseconds whatever it still saves. Release even when a save failed.
    local saved, failure = pcall(save)
    if not saved then
        log('the save failed: ' .. tostring(failure))
    end
    local ok, refused = exports[GetConvar('observer_bridge', '')]:releaseShutdown()
    log(ok and 'released the stop' or ('the stop was not released: ' .. tostring(refused)))
end)

AddEventHandler('observer:startChanges', function(payload)
    log('startChanges ' .. fields(payload, { 'at', 'previousAt', 'baseline', 'skipped', 'truncated' }))
    local counts = payload.counts
    log('  ' .. fields(counts, { 'added', 'removed', 'updated', 'sameVersion', 'renamed', 'moved',
        'didNotStart', 'pending' }))
    for _, resource in ipairs(payload.resources) do
        log(('  %s %s %s -> %s'):format(resource.name, resource.change, show(resource.from), show(resource.to)))
    end
    -- Your changelog goes here: post what changed since the previous start to your staff's channel.
end)

AddEventHandler('observer:playerRefused', function(payload)
    log('playerRefused ' .. fields(payload, { 'ban', 'reason', 'endsAt', 'by', 'name', 'license' }))
    -- Your staff's log goes here: a banned player tried to connect and was refused.
end)

AddEventHandler('observer:playerBanned', function(payload)
    log('playerBanned ' .. fields(payload, { 'player', 'ban', 'reason', 'endsAt', 'by', 'name', 'license' }))
    -- Your saves go here: player payload.player is dropped right after this event.
end)

-- Your framework gives the label where it knows the player's character, once they picked it for
-- instance, and clears it when they go back to the character menu. To try it from the server
-- console: `observer_example_label 1 CHAR-1234 John Doe` labels player 1, `observer_example_label 1`
-- clears the label.
RegisterCommand('observer_example_label', function(source, args)
    if source ~= 0 then
        return
    end
    local bridge = GetConvar('observer_bridge', '')
    local ok, refused
    if args[2] then
        ok, refused = exports[bridge]:setPlayerLabel(args[1], { id = args[2], name = table.concat(args, ' ', 3) })
    else
        ok, refused = exports[bridge]:clearPlayerLabel(args[1])
    end
    if ok then
        log('label sent to Observer')
    else
        log('label refused: ' .. tostring(refused))
    end
end, true)
