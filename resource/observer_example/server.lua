-- Listens to the server events of Observer's integration contract (resource/CONTRACT.md) and prints
-- each one with its fields, and gives players labels for Observer's panel from the console. It
-- shows nothing to players: the comments mark where your own notifications, cancellation notices,
-- saves, labels and the staff's announcements and messages would go.

-- The contract this resource was written for.
local CONTRACT_VERSION = 5

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
end)

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
    log('shuttingDown ' .. fields(payload, { 'delayMs', 'reason', 'by' }))
    -- Your saves go here: FXServer quits in payload.delayMs milliseconds.
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

AddEventHandler('observer:playerKicked', function(payload)
    log('playerKicked ' .. fields(payload, { 'player', 'reason', 'by', 'cause' }))
    -- Your saves go here: player payload.player is still connected, and dropped once the handlers
    -- of the kick have run.
end)

AddEventHandler('observer:announcement', function(payload)
    log('announcement ' .. fields(payload, { 'by', 'message' }))
    -- What shows it to everyone goes here, a line of FXServer's chat (`chat:addMessage`) or your own
    -- notification: Observer shows players nothing of an announcement.
end)

AddEventHandler('observer:directMessage', function(payload)
    log('directMessage ' .. fields(payload, { 'player', 'by', 'message' }))
    -- What shows it to payload.player alone goes here, the same way, while
    -- GetPlayerName(payload.player) still answers: they may have left since the staff wrote.
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
