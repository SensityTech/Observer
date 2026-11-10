-- Counts what a resource sends and receives, for Observer's Debug pages: events, state bag writes
-- and outgoing HTTP requests. Names, counts and sizes: never a payload, a value, a path or a query.
--
-- This file is no script of the bridge, and the manifest does not list it. While the owner keeps
-- Debug on, Observer adds two lines to the build's citizen/scripting/lua/scheduler.lua at each
-- start: the first loads this file into every server Lua runtime, one per resource, and keeps the
-- table it returns; the last calls that table's `init` with the scheduler's event handlers.
--
-- It runs inside other people's resources, so: it returns its table and never raises at the top
-- level, creates no global, allocates nothing where an event passes, never calls a resource's code
-- under pcall (an error must read as it always did), and never looks into a resource's functions.

local bridge = ...

local WIRE_TYPE = 'observer_debug'
local WIRE_VERSION = 1
local HOOK_VERSION = '1'
local BRIDGE_CONVAR = 'observer_bridge'
-- What is counted under once a window holds more distinct names than it keeps.
local OVERFLOW = '__overflow__'
local FLUSH_MS = 1000
local MAX_FLUSH_ERRORS = 10
local MAX_EVENTS = 400
local MAX_SENDERS = 200
local MAX_REFUSED = 200
local MAX_BAGS = 200
local MAX_HOSTS = 50
local LARGEST = 10
-- A length no script meant: the string's own is taken instead.
local MAX_LENGTH = 4294967296

-- Taken before any resource runs, which may replace them.
local _G = _G
local Citizen = Citizen
local type = type
local pairs = pairs
local next = next
local tostring = tostring
local tonumber = tonumber
local rawget = rawget
local rawset = rawset
local pcall = pcall
local str_byte = string.byte
local str_sub = string.sub
local str_match = string.match
local str_upper = string.upper
local str_lower = string.lower
local math_floor = math.floor
local osLib = rawget(_G, 'os')
local citizenWait = type(Citizen) == 'table' and Citizen.Wait or nil
local createThread = type(Citizen) == 'table' and Citizen.CreateThread or nil

local idle = { init = function() end }

-- The server alone, and only the one Observer started with this very bridge: the same build may
-- run elsewhere, where nobody reads the reports.
local function startedByObserver()
    return IsDuplicityVersion() and GetConvar(BRIDGE_CONVAR, '') == bridge
end
local asked, mine = pcall(startedByObserver)
if not asked or not mine or type(bridge) ~= 'string' or type(Citizen) ~= 'table' then
    return idle
end

-- The bridge is Observer's own: what it sends and receives is not the server's.
local function inTheBridge()
    return GetCurrentResourceName() == bridge
end
local named, own = pcall(inTheBridge)
if named and own then
    return idle
end

-- The clock stays in its own unit where an event passes; microseconds are worked out at each report.
local clock, clockToUs, timerKind
do
    local function probe(fn)
        if type(fn) ~= 'function' then
            return false
        end
        local ok, value = pcall(fn)
        return ok and type(value) == 'number'
    end
    local function gameTimer()
        return GetGameTimer()
    end
    local hasOs = type(osLib) == 'table'
    if hasOs and probe(osLib.nanotime) then
        clock, clockToUs, timerKind = osLib.nanotime, 0.001, 'nanotime'
    elseif hasOs and probe(osLib.microtime) then
        clock, clockToUs, timerKind = osLib.microtime, 1, 'microtime'
    elseif hasOs and probe(osLib.clock) then
        clock, clockToUs, timerKind = osLib.clock, 1000000, 'clock'
    elseif probe(gameTimer) then
        clock, clockToUs, timerKind = gameTimer, 1000, 'gametimer'
    else
        clock, clockToUs, timerKind = function()
            return 0
        end, 1, 'none'
    end
end

-- Every wrapper passes straight through until `init`, and again once reporting broke.
local enabled = false
local initialized = false
local eventHandlers = nil
local jsonEncode = nil
local natives = {}
local seq = 0
local lastFlush = 0
local helloPending = true

-- How deep the events dispatched inside one another are, to take a handler's own time alone.
local depth = 0
local childTime = {}

-- The window being counted, emptied at each report.
local events, eventKeys -- events[way][name] = { count, bytes, largest, time, longest }
local senders, senderKeys -- senders[name][player] = { count, bytes }
local refused, refusedKeys -- refused[name][player] = count
local bags, bagKeys -- bags[kind][key] = { count, bytes, largest, replicated }
local largest, largestCount, smallest -- { bag, key, bytes, replicated }, the smallest one's index
local hosts, hostKeys -- hosts[method][host] = { count, bytes }
local overflow

local function emptyWindow()
    events, eventKeys = {}, 0
    senders, senderKeys = {}, 0
    refused, refusedKeys = {}, 0
    bags, bagKeys = {}, 0
    largest, largestCount, smallest = {}, 0, 1
    hosts, hostKeys = {}, 0
    overflow = 0
end
emptyWindow()

local function global(name)
    -- Through the natives loader's __index when the native was not asked for yet.
    return _G[name]
end

local function native(name)
    local fn = natives[name]
    if fn == nil then
        local ok, value = pcall(global, name)
        if ok and type(value) == 'function' then
            fn = value
            natives[name] = value
        end
    end
    return fn
end

local function lengthOf(length, data)
    if type(length) == 'number' and length >= 0 and length < MAX_LENGTH then
        return length
    end
    return type(data) == 'string' and #data or 0
end

-- A player's id as an integer, or nil when it is no id.
local function playerOf(value)
    local id = tonumber(value)
    if id == nil or not (id >= -1 and id < MAX_LENGTH) then
        return nil
    end
    return math_floor(id)
end

-- The counts of an event, and the name they are kept under.
local function eventOf(way, name)
    local byName = events[way]
    if byName == nil then
        byName = {}
        events[way] = byName
    end
    local counts = byName[name]
    if counts == nil then
        if eventKeys >= MAX_EVENTS then
            overflow = overflow + 1
            name = OVERFLOW
            counts = byName[name]
        end
        if counts == nil then
            counts = { 0, 0, 0, 0, 0 }
            byName[name] = counts
            eventKeys = eventKeys + 1
        end
    end
    return counts, name
end

local function countSender(name, player, length)
    local byPlayer = senders[name]
    local counts = byPlayer and byPlayer[player]
    if counts == nil then
        if senderKeys >= MAX_SENDERS then
            overflow = overflow + 1
            name = OVERFLOW
            byPlayer = senders[name]
            counts = byPlayer and byPlayer[player]
        end
        if counts == nil then
            if byPlayer == nil then
                byPlayer = {}
                senders[name] = byPlayer
            end
            counts = { 0, 0 }
            byPlayer[player] = counts
            senderKeys = senderKeys + 1
        end
    end
    counts[1] = counts[1] + 1
    counts[2] = counts[2] + length
end

local function countRefused(name, player)
    local byPlayer = refused[name]
    local count = byPlayer and byPlayer[player]
    if count == nil then
        if refusedKeys >= MAX_REFUSED then
            overflow = overflow + 1
            name = OVERFLOW
            byPlayer = refused[name]
            count = byPlayer and byPlayer[player]
        end
        if count == nil then
            if byPlayer == nil then
                byPlayer = {}
                refused[name] = byPlayer
            end
            count = 0
            refusedKeys = refusedKeys + 1
        end
    end
    byPlayer[player] = count + 1
end

local function keepLargest(bag, key, length, replicated)
    if largestCount < LARGEST then
        largestCount = largestCount + 1
        largest[largestCount] = { bag, key, length, replicated }
    else
        local least = largest[smallest]
        if length <= least[3] then
            return
        end
        least[1], least[2], least[3], least[4] = bag, key, length, replicated
    end
    local index, least = 1, largest[1][3]
    for i = 2, largestCount do
        if largest[i][3] < least then
            index, least = i, largest[i][3]
        end
    end
    smallest = index
end

local function countSend(one, all, name, target, payload, payloadLength)
    local length = lengthOf(payloadLength, payload)
    local counts = eventOf(playerOf(target) == -1 and all or one, type(name) == 'string' and name or tostring(name))
    counts[1] = counts[1] + 1
    counts[2] = counts[2] + length
    if length > counts[3] then
        counts[3] = length
    end
end

local function countBagWrite(bagName, keyName, value, valueLength, replicated)
    local length = lengthOf(valueLength, value)
    local bag = type(bagName) == 'string' and bagName or tostring(bagName)
    local key = type(keyName) == 'string' and keyName or tostring(keyName)
    local sent = (replicated == true or (type(replicated) == 'number' and replicated ~= 0)) and 1 or 0

    local kind = 'other'
    local first = str_byte(bag, 1)
    if first == 103 and bag == 'global' then
        kind = 'global'
    elseif first == 112 and str_sub(bag, 1, 7) == 'player:' then
        kind = 'player'
    elseif first == 101 and str_sub(bag, 1, 7) == 'entity:' then
        kind = 'entity'
    elseif first == 108 and str_sub(bag, 1, 12) == 'localEntity:' then
        kind = 'localEntity'
    end

    local byKey = bags[kind]
    if byKey == nil then
        byKey = {}
        bags[kind] = byKey
    end
    local name = key
    local counts = byKey[name]
    if counts == nil then
        if bagKeys >= MAX_BAGS then
            overflow = overflow + 1
            name = OVERFLOW
            counts = byKey[name]
        end
        if counts == nil then
            counts = { 0, 0, 0, 0 }
            byKey[name] = counts
            bagKeys = bagKeys + 1
        end
    end
    counts[1] = counts[1] + 1
    counts[2] = counts[2] + length
    if length > counts[3] then
        counts[3] = length
    end
    counts[4] = counts[4] + sent

    if largestCount < LARGEST or length > largest[smallest][3] then
        keepLargest(bag, key, length, sent)
    end
end

local function countRequest(request)
    if type(request) ~= 'table' then
        return
    end
    local method = request.method
    if type(method) == 'string' and #method > 0 and #method <= 16 then
        method = str_upper(method)
    else
        method = 'GET'
    end

    -- The host alone: a path or a query may hold a webhook's token or a key.
    local host = '?'
    local url = request.url
    if type(url) == 'string' then
        local authority = str_match(url, '^%a[%w+.-]*://([^/?#]*)')
        if authority ~= nil then
            authority = str_match(authority, '^.*@(.*)$') or authority
            local name = str_match(authority, '^%[([^%]]*)%]') or str_match(authority, '^([^:]*)')
            if name ~= nil and name ~= '' then
                host = str_lower(name)
            end
        end
    end

    local body = request.data
    local length = type(body) == 'string' and #body or 0
    local byHost = hosts[method]
    if byHost == nil then
        byHost = {}
        hosts[method] = byHost
    end
    local counts = byHost[host]
    if counts == nil then
        if hostKeys >= MAX_HOSTS then
            overflow = overflow + 1
            host = OVERFLOW
            counts = byHost[host]
        end
        if counts == nil then
            counts = { 0, 0 }
            byHost[host] = counts
            hostKeys = hostKeys + 1
        end
    end
    counts[1] = counts[1] + 1
    counts[2] = counts[2] + length
end

-- Passes on what the dispatch returned, untouched and without a table.
local function dispatched(started, level, counts, ...)
    local total = clock() - started
    depth = level - 1
    if level > 1 then
        childTime[level - 1] = (childTime[level - 1] or 0) + total
    end
    local own = total - (childTime[level] or 0)
    if own < 0 then
        own = 0
    end
    counts[4] = counts[4] + own
    if own > counts[5] then
        counts[5] = own
    end
    return ...
end

local function triggered(started, level, counts, ...)
    local total = clock() - started
    depth = level - 1
    if level > 1 then
        childTime[level - 1] = (childTime[level - 1] or 0) + total
    end
    counts[4] = counts[4] + total
    if total > counts[5] then
        counts[5] = total
    end
    return ...
end

-- The scheduler's own routine, counted on its way in. The source is told apart as the scheduler
-- tells it: `net:<id>` from a client, `internal-net:<id>` from the engine, anything else local.
local function countedRoutine(routine)
    return function(eventName, eventPayload, eventSource)
        if not enabled then
            return routine(eventName, eventPayload, eventSource)
        end

        local way = 'in'
        local player = nil
        if type(eventSource) == 'string' then
            local first = str_byte(eventSource, 1)
            if first == 110 and str_sub(eventSource, 1, 3) == 'net' then
                way = 'inN'
                player = playerOf(str_sub(eventSource, 5))
            elseif first == 105 and str_sub(eventSource, 1, 12) == 'internal-net' then
                way = 'inI'
            end
        end

        local name = eventName
        if type(name) ~= 'string' then
            name = tostring(name)
        end
        if eventHandlers ~= nil then
            local entry = rawget(eventHandlers, eventName)
            local handlers = type(entry) == 'table' and entry.handlers or nil
            if handlers == nil then
                return routine(eventName, eventPayload, eventSource)
            end
            if way == 'inN' and not entry.safeForNet then
                -- The scheduler is about to refuse it: no resource opened this event to clients.
                countRefused(name, player or -1)
            elseif type(handlers) ~= 'table' or next(handlers) == nil then
                return routine(eventName, eventPayload, eventSource)
            end
        end

        local length = type(eventPayload) == 'string' and #eventPayload or 0
        local counts
        counts, name = eventOf(way, name)
        counts[1] = counts[1] + 1
        counts[2] = counts[2] + length
        if length > counts[3] then
            counts[3] = length
        end
        if player ~= nil then
            countSender(name, player, length)
        end

        local level = depth + 1
        depth = level
        childTime[level] = 0
        return dispatched(clock(), level, counts, routine(eventName, eventPayload, eventSource))
    end
end

-- Each wrapper passes on any argument a later scheduler may add.
local function countedTriggerEvent(original)
    return function(eventName, eventPayload, payloadLength, ...)
        if not enabled then
            return original(eventName, eventPayload, payloadLength, ...)
        end
        local length = lengthOf(payloadLength, eventPayload)
        local counts = eventOf('loc', type(eventName) == 'string' and eventName or tostring(eventName))
        counts[1] = counts[1] + 1
        counts[2] = counts[2] + length
        if length > counts[3] then
            counts[3] = length
        end
        local level = depth + 1
        depth = level
        childTime[level] = 0
        return triggered(clock(), level, counts, original(eventName, eventPayload, payloadLength, ...))
    end
end

local function countedTriggerClientEvent(original)
    return function(eventName, eventTarget, eventPayload, payloadLength, ...)
        if enabled then
            countSend('toC', 'toA', eventName, eventTarget, eventPayload, payloadLength)
        end
        return original(eventName, eventTarget, eventPayload, payloadLength, ...)
    end
end

local function countedTriggerLatentClientEvent(original)
    return function(eventName, eventTarget, eventPayload, payloadLength, ...)
        if enabled then
            countSend('lat', 'latA', eventName, eventTarget, eventPayload, payloadLength)
        end
        return original(eventName, eventTarget, eventPayload, payloadLength, ...)
    end
end

local function countedSetStateBagValue(original)
    return function(bagName, keyName, value, valueLength, replicated, ...)
        if enabled then
            countBagWrite(bagName, keyName, value, valueLength, replicated)
        end
        return original(bagName, keyName, value, valueLength, replicated, ...)
    end
end

local function countedHttpRequest(original)
    return function(request, ...)
        if enabled then
            countRequest(request)
        end
        return original(request, ...)
    end
end

local wrapped = {
    { name = 'TriggerEventInternal', count = countedTriggerEvent, done = false },
    { name = 'TriggerClientEventInternal', count = countedTriggerClientEvent, done = false },
    { name = 'TriggerLatentClientEventInternal', count = countedTriggerLatentClientEvent, done = false },
    { name = 'SetStateBagValue', count = countedSetStateBagValue, done = false },
    { name = 'PerformHttpRequestInternalEx', count = countedHttpRequest, done = false },
}
local retried = false

local function wrapNatives()
    for i = 1, #wrapped do
        local one = wrapped[i]
        if not one.done then
            local ok, original = pcall(global, one.name)
            if ok and type(original) == 'function' then
                rawset(_G, one.name, one.count(original))
                one.done = true
            end
        end
    end
end

local function encoder()
    local jsonLib = rawget(_G, 'json')
    if type(jsonLib) == 'table' and type(jsonLib.encode) == 'function' then
        return jsonLib.encode
    end
    return nil
end

local function microseconds(value)
    return math_floor(value * clockToUs)
end

local function flush()
    if not retried then
        -- A native the runtime had not loaded at `init` is there by the first report.
        retried = true
        wrapNatives()
    end
    depth = 0

    local now = clock()
    local elapsed = now - lastFlush
    lastFlush = now
    -- Taken first, so that an error below cannot make the same window fail again.
    local wEvents, wSenders, wRefused, wBags, wLargest, wLargestCount, wHosts, wOverflow =
        events, senders, refused, bags, largest, largestCount, hosts, overflow
    emptyWindow()

    local hello = helloPending
    if not hello and wOverflow == 0 and next(wEvents) == nil and next(wSenders) == nil
        and next(wRefused) == nil and next(wBags) == nil and next(wHosts) == nil
    then
        return
    end

    local report = {
        type = WIRE_TYPE,
        v = WIRE_VERSION,
        seq = seq + 1,
        dur = math_floor(elapsed * clockToUs / 1000),
    }

    local list, count = {}, 0
    for way, byName in pairs(wEvents) do
        for name, counts in pairs(byName) do
            count = count + 1
            list[count] = {
                way, name, counts[1], math_floor(counts[2]), math_floor(counts[3]),
                microseconds(counts[4]), microseconds(counts[5]),
            }
        end
    end
    if count > 0 then
        report.ev = list
    end

    list, count = {}, 0
    for name, byPlayer in pairs(wSenders) do
        for player, counts in pairs(byPlayer) do
            count = count + 1
            list[count] = { name, player, counts[1], math_floor(counts[2]) }
        end
    end
    if count > 0 then
        report.np = list
    end

    list, count = {}, 0
    for name, byPlayer in pairs(wRefused) do
        for player, times in pairs(byPlayer) do
            count = count + 1
            list[count] = { name, player, times }
        end
    end
    if count > 0 then
        report.un = list
    end

    list, count = {}, 0
    for kind, byKey in pairs(wBags) do
        for key, counts in pairs(byKey) do
            count = count + 1
            list[count] = { kind, key, counts[1], math_floor(counts[2]), math_floor(counts[3]), counts[4] }
        end
    end
    if count > 0 then
        report.sb = list
    end

    if wLargestCount > 0 then
        list = {}
        for i = 1, wLargestCount do
            local write = wLargest[i]
            list[i] = { write[1], write[2], math_floor(write[3]), write[4] }
        end
        report.sbl = list
    end

    list, count = {}, 0
    for method, byHost in pairs(wHosts) do
        for host, counts in pairs(byHost) do
            count = count + 1
            list[count] = { method, host, counts[1], math_floor(counts[2]) }
        end
    end
    if count > 0 then
        report.http = list
    end

    if wOverflow > 0 then
        report.ovf = wOverflow
    end
    if hello then
        report.hello = { hook = HOOK_VERSION, timer = timerKind }
    end

    local encode = jsonEncode or encoder()
    local trace = native('PrintStructuredTrace')
    if encode == nil or trace == nil then
        error('json.encode or PrintStructuredTrace is missing')
    end
    trace(encode(report))
    seq = seq + 1
    if hello then
        helloPending = false
    end
end

local function flushForever()
    local wait = citizenWait
    if type(wait) ~= 'function' then
        return
    end
    local errors = 0
    while true do
        wait(FLUSH_MS)
        if pcall(flush) then
            errors = 0
        else
            errors = errors + 1
            if errors >= MAX_FLUSH_ERRORS then
                -- Nothing would ever report what the wrappers count: they pass through again.
                enabled = false
                emptyWindow()
                return
            end
        end
    end
end

-- Called by the line Observer adds at the end of scheduler.lua, once the scheduler defined
-- everything.
local function init(context)
    if initialized then
        return
    end
    initialized = true
    if type(context) == 'table' and type(context.eventHandlers) == 'table' then
        eventHandlers = context.eventHandlers
    end

    -- Now, before a resource's own script can replace them.
    jsonEncode = encoder()
    native('PrintStructuredTrace')
    wrapNatives()
    if type(citizenWait) ~= 'function' then
        citizenWait = Citizen.Wait
    end
    if type(createThread) ~= 'function' then
        createThread = Citizen.CreateThread
    end
    -- Without its thread nothing could be reported: every wrapper keeps passing through.
    if type(citizenWait) ~= 'function' or type(createThread) ~= 'function' then
        return
    end
    lastFlush = clock()
    enabled = true
    createThread(flushForever)
end

-- Now, before the scheduler hands the runtime its routine: that one alone is wrapped, and the
-- function is put back at once.
local setEventRoutine = Citizen.SetEventRoutine
if type(setEventRoutine) == 'function' then
    Citizen.SetEventRoutine = function(routine)
        Citizen.SetEventRoutine = setEventRoutine
        if type(routine) ~= 'function' then
            return setEventRoutine(routine)
        end
        return setEventRoutine(countedRoutine(routine))
    end
end

return { init = init }
