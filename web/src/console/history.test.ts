import { beforeEach, describe, expect, it } from 'vitest';

import { newer, older, readHistory, remember, START } from './history';

describe('console history', () => {
    beforeEach(() => localStorage.clear());

    it('should be kept per user, once per repeated command', () => {
        let history = remember('ada', [], 'status');
        history = remember('ada', history, 'status');
        history = remember('ada', history, 'ensure chat');
        remember('grace', [], 'refresh');

        expect(readHistory('ada')).toEqual(['status', 'ensure chat']);
        expect(readHistory('grace')).toEqual(['refresh']);
        expect(readHistory('kenji')).toEqual([]);
    });

    it('should leave out a command Observer masked a secret in', () => {
        const history = remember('ada', ['status'], 'set rcon_password"letmein"', true);

        expect(history).toEqual(['status']);
        expect(readHistory('ada')).toEqual([]);
    });

    it('should forget what it cannot read', () => {
        localStorage.setItem('observer.console.history.ada', '{"not":"a list"');
        expect(readHistory('ada')).toEqual([]);
        localStorage.setItem('observer.console.history.ada', '["status", 3, null]');
        expect(readHistory('ada')).toEqual(['status']);
    });

    it('should walk back with up and forward with down to what was being typed', () => {
        const history = ['status', 'ensure chat', 'restart chat'];

        const [first, newest] = older(history, START, 'say hel');
        const [second, before] = older(history, first, newest);
        const [third, oldest] = older(history, second, before);
        const [still, stillOldest] = older(history, third, oldest);
        const [back, next] = newer(history, still, stillOldest);
        const [, last] = newer(history, newer(history, back, next)[0], 'restart chat');

        expect([newest, before, oldest, stillOldest]).toEqual([
            'restart chat',
            'ensure chat',
            'status',
            'status',
        ]);
        expect(next).toBe('ensure chat');
        expect(last).toBe('say hel');
        expect(older([], START, 'typed')).toEqual([START, 'typed']);
        expect(newer(history, START, 'typed')).toEqual([START, 'typed']);
    });
});
