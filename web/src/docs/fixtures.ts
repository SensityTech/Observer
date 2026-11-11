// The contract, the bridge and the example as the repository holds them, for the tests: what the
// API serves from the binary, read here from the same files.

import bridge from '../../../resource/observer/server.lua?raw';
import contract from '../../../resource/CONTRACT.md?raw';
import exampleManifest from '../../../resource/observer_example/fxmanifest.lua?raw';
import exampleScript from '../../../resource/observer_example/server.lua?raw';
import type { IntegrationDocs } from '../api/generated/IntegrationDocs';
import type { IntegrationServer } from '../api/generated/IntegrationServer';

const named = /local EVENTS = \{\n([\s\S]*?)\n\}/.exec(bridge)?.[1] ?? '';

/**
 * What `GET /api/docs/integration` answers for a server that does not run, or for an account that
 * is not told what the server does when `server` is null.
 */
export const integrationDocs = (server: Partial<IntegrationServer> | null = {}): IntegrationDocs => ({
    contract,
    contractVersion: Number(/local CONTRACT_VERSION = (\d+)/.exec(bridge)?.[1]),
    events: [
        ...new Set([
            ...[...named.matchAll(/^\s*(\w+) = true,$/gm)].map((match) => `observer:${match[1] ?? ''}`),
            ...[...bridge.matchAll(/TriggerEvent\('(observer:\w+)'/g)].map((match) => match[1] ?? ''),
        ]),
    ],
    exports: [...bridge.matchAll(/^exports\('(\w+)', function\(([^)]*)\)/gm)].map((match) => ({
        name: match[1] ?? '',
        parameters: (match[2] ?? '')
            .split(',')
            .map((parameter) => parameter.trim())
            .filter(Boolean),
    })),
    example: [
        { name: 'observer_example/fxmanifest.lua', text: exampleManifest },
        { name: 'observer_example/server.lua', text: exampleScript },
    ],
    server:
        server === null
            ? null
            : { bridge: null, starting: false, txadminEvents: null, txadminEventsNext: null, ...server },
});
