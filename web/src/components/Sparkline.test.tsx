import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Sparkline, sparklinePath } from './Sparkline';

describe('sparklinePath', () => {
    it('should span the box from the lowest value to the highest', () => {
        expect(sparklinePath([0, 10, 5])).toEqual({
            line: 'M0 26L50 2L100 14',
            area: 'M0 28L0 26L50 2L100 14L100 28Z',
            flat: false,
        });
    });

    it('should break the path at a gap and draw a lone point as a dot', () => {
        const path = sparklinePath([1, 2, null, 3, null]);

        expect(path?.line).toBe('M0 26L25 14M75 2h0.01');
        expect(path?.area).toBe('M0 28L0 26L25 14L25 28Z');
    });

    it('should scale to a shared maximum and call a constant trend flat', () => {
        expect(sparklinePath([0, 50], 100)?.line).toBe('M0 26L100 14');
        expect(sparklinePath([4, 4, 4])?.flat).toBe(true);
        expect(sparklinePath([null, null])).toBeNull();
    });
});

describe('Sparkline', () => {
    it('should name its series unless the trend is flat', () => {
        const { container, rerender } = render(
            <Sparkline values={[1, 3, 2]} label="Players over the last hour" />,
        );
        expect(container.querySelector('svg')?.getAttribute('aria-label')).toBe('Players over the last hour');

        rerender(<Sparkline values={[2, 2]} label="Players over the last hour" />);
        expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    });
});
