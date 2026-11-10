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

/** Whether a mark placed there lies out of the 800 by 600 map. */
const outside = (style: CSSStyleDeclaration) =>
    parseFloat(style.left) < 0 ||
    parseFloat(style.left) > 800 ||
    parseFloat(style.top) < 0 ||
    parseFloat(style.top) > 600;

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

    it('should draw Rockstar’s map as tiles in place of the grid, and tell when its servers do not send it', async () => {
        const onTilesFailed = vi.fn();
        draw({ tiles: 'atlas', onTilesFailed });
        await act(async () => {});
        const tiles = [...document.querySelectorAll('img')];

        expect(tiles.length).toBeGreaterThan(1);
        expect(tiles[0]!.src).toBe('https://s.rsg.sc/sc/images/games/GTAV/map/game/0/0/0.jpg');
        for (const tile of tiles) {
            expect(tile.src).toMatch(
                /^https:\/\/s\.rsg\.sc\/sc\/images\/games\/GTAV\/map\/game\/\d\/\d+\/\d+\.jpg$/,
            );
            expect(tile.getAttribute('referrerpolicy')).toBe('no-referrer');
        }
        expect(screen.queryAllByText('0')).toHaveLength(0);
        expect(screen.getByRole('button', { name: DOTS[0]!.label })).not.toBeNull();

        // A tile that does not come leaves the whole map under it; the whole map missing is told.
        fireEvent.error(tiles[1]!);
        expect(tiles[1]!.style.visibility).toBe('hidden');
        expect(onTilesFailed).not.toHaveBeenCalled();
        fireEvent.error(tiles[0]!);
        expect(onTilesFailed).toHaveBeenCalledTimes(1);
    });

    it('should bring a mark out of view into it when the keyboard reaches it', async () => {
        draw({ tiles: 'atlas' });
        await act(async () => {});
        const mark = screen.getByRole('button', { name: DOTS[0]!.label });
        for (let click = 0; click < 8; click += 1) {
            fireEvent.click(screen.getByRole('button', { name: en.map.zoomIn }));
        }
        expect(outside(mark.parentElement!.style)).toBe(true);

        fireEvent.focus(mark);

        expect(parseFloat(mark.parentElement!.style.left)).toBeCloseTo(400, 0);
        expect(parseFloat(mark.parentElement!.style.top)).toBeCloseTo(300, 0);
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
