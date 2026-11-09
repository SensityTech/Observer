import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { MapCanvas, type Dot } from './MapCanvas';

const DOTS: Dot[] = [
    { id: 3, x: 412, y: -1208, tier: 'regular', name: 'Ruby Vex', label: 'Ruby Vex, Regular, 23h 40m' },
    { id: 7, x: -500, y: 2000, tier: 'new', name: 'kaito', label: 'kaito, New, 2h 5m' },
];

/** jsdom lays nothing out: every map is 800 by 600. */
class FixedSize {
    constructor(private readonly report: ResizeObserverCallback) {}
    observe() {
        this.report([{ contentRect: { width: 800, height: 600 } } as ResizeObserverEntry], this as never);
    }
    disconnect() {}
}

const draw = (props: Partial<Parameters<typeof MapCanvas>[0]> = {}) => {
    const onSelect = vi.fn();
    render(
        <I18nProvider>
            <MapCanvas
                image={null}
                bounds={null}
                dots={DOTS}
                selected={null}
                names={false}
                focus={null}
                onSelect={onSelect}
                {...props}
            />
        </I18nProvider>,
    );
    return onSelect;
};

describe('MapCanvas', () => {
    beforeEach(() => {
        vi.stubGlobal('ResizeObserver', FixedSize);
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should give each player a button named by their name, colour and play time', async () => {
        const onSelect = draw();
        await act(async () => {});

        fireEvent.click(screen.getByRole('button', { name: DOTS[0]!.label }));

        expect(screen.getByRole('button', { name: DOTS[1]!.label })).not.toBeNull();
        expect(onSelect).toHaveBeenCalledWith(3);
        expect(screen.queryByText('Ruby Vex')).toBeNull();
    });

    it('should name the selected player, or every player when asked', async () => {
        draw({ selected: 7 });
        await act(async () => {});

        expect(screen.getByText('kaito')).not.toBeNull();
        expect(screen.queryByText('Ruby Vex')).toBeNull();
        expect(screen.getByRole('button', { name: DOTS[1]!.label }).getAttribute('aria-pressed')).toBe(
            'true',
        );
        cleanup();

        draw({ names: true });
        await act(async () => {});
        expect(screen.getByText('Ruby Vex')).not.toBeNull();
        expect(screen.getByText('kaito')).not.toBeNull();
    });

    it('should zoom from its buttons and draw a grid of game coordinates without an image', async () => {
        draw();
        await act(async () => {});
        const scaleBefore = screen.getByText(/ m$/).textContent;

        fireEvent.click(screen.getByRole('button', { name: en.map.zoomIn }));
        fireEvent.click(screen.getByRole('button', { name: en.map.zoomIn }));
        fireEvent.click(screen.getByRole('button', { name: en.map.zoomIn }));

        expect(screen.getByText(/ m$/).textContent).not.toBe(scaleBefore);
        expect(screen.getAllByText('0').length).toBeGreaterThan(0);
    });
});
