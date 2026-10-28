import { describe, expect, it } from 'vitest';

import { parseServerMessage } from './messages';

const parse = (message: unknown) => parseServerMessage(JSON.stringify(message));

describe('parseServerMessage', () => {
    it('should accept every message of the contract', () => {
        for (const message of [
            { type: 'hello', connectionId: 3, contract: 1 },
            {
                type: 'snapshot',
                topic: 'status',
                data: { version: '0.1.0', startedAt: '2026-10-28T09:00:00Z' },
            },
            { type: 'snapshot', topic: 'audit', data: { latestId: null } },
            { type: 'resync', topic: 'audit' },
            { type: 'unsubscribed', topic: 'status', reason: 'forbidden' },
            { type: 'pong' },
            { type: 'error', code: 'invalid_request' },
        ]) {
            expect(parse(message)).toEqual(message);
        }
    });

    it('should refuse what only looks like it', () => {
        for (const message of [
            null,
            [],
            { type: 'hello', connectionId: '3', contract: 1 },
            { type: 'snapshot', topic: 'status', data: { version: '0.1.0', startedAt: 'yesterday' } },
            { type: 'snapshot', topic: 'audit' },
            { type: 'unsubscribed', topic: 'audit', reason: 'bored' },
            { type: 'resync', topic: 'console' },
            { type: 'shutdown' },
        ]) {
            expect(parse(message)).toBeNull();
        }
        expect(parseServerMessage('{"type":')).toBeNull();
    });
});
