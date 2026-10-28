-- Listens to the server events of Observer's integration contract (resource/CONTRACT.md) and prints
-- each one with its fields. It shows nothing to players: the comments mark where your own
-- notifications, cancellation notices and saves would go.

-- The contract this resource was written for.
local CONTRACT_VERSION = 1

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
    if GetResourceState('observer') ~= 'started' then
        log('^3the observer bridge is not running: is this server started by Observer?^7')
        return
    end
    local version = exports.observer:getContractVersion()
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
