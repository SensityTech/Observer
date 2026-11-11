import { describe, expect, it } from 'vitest';

import { fill } from './fill';

describe('fill', () => {
    it('should fill each placeholder it knows once and leave every other brace as written', () => {
        const text = fill('Hello {name}. ID: {id} {id} {reason} {', { id: '7KQ-2XP', name: 'kaito {id}' });

        expect(text).toBe('Hello kaito {id}. ID: 7KQ-2XP 7KQ-2XP {reason} {');
    });

    it('should not take a property every object has for a placeholder', () => {
        expect(fill('{toString} {name}', { name: 'kaito' })).toBe('{toString} kaito');
    });
});
