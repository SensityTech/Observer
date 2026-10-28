import { describe, expect, it } from 'vitest';

import { cn } from './cn';

describe('cn', () => {
    it('should keep a size and a colour of the custom text scale side by side', () => {
        expect(cn('text-ui text-ink-3')).toBe('text-ui text-ink-3');
    });

    it('should let a later class win over a conflicting custom one', () => {
        expect(cn('rounded-control', 'rounded-sm')).toBe('rounded-sm');
        expect(cn('text-body', 'text-ui')).toBe('text-ui');
        expect(cn('shadow-sm', 'shadow-btn')).toBe('shadow-btn');
    });
});
