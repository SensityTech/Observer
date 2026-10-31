import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { FALLING, FEW, RISING, STEADY, SUMMARY, SUMMARY_DAY } from './fixtures';
import { signedPercent, trendWindows, VerdictChip, verdictDetails, verdictText, verdictTone } from './words';

const MINUS = '−';

describe('verdictText', () => {
    it('should give a called change its sign, a fall the minus sign', () => {
        expect(verdictText(en, 'en', RISING)).toBe('Rising +38%');
        expect(verdictText(en, 'en', FALLING)).toBe(`Falling ${MINUS}20%`);
    });

    it('should never put a figure on a change the test did not call', () => {
        // Both carry a change; neither is a verdict on it.
        expect(STEADY.change).not.toBeNull();
        expect(FEW.change).not.toBeNull();
        expect(verdictText(en, 'en', STEADY)).toBe('No clear change');
        expect(verdictText(en, 'en', FEW)).toBe('Not enough drops');
    });

    it('should warn for a rise alone', () => {
        expect(verdictTone('rising')).toBe('warning');
        expect(verdictTone('falling')).toBe('neutral');
        expect(verdictTone('noClearChange')).toBe('neutral');
        expect(verdictTone('notEnoughDrops')).toBe('neutral');
    });
});

describe('signedPercent', () => {
    it('should sign either way with the plus and the minus sign, and leave zero bare', () => {
        expect(signedPercent('en', 0.384)).toBe('+38%');
        expect(signedPercent('en', -0.2)).toBe(`${MINUS}20%`);
        expect(signedPercent('en', 0)).toBe('0%');
        expect(signedPercent('en', 12.5)).toBe('+1,250%');
    });
});

describe('verdictDetails', () => {
    it('should give the interval, both rates, the drops counted and the windows', () => {
        expect(verdictDetails(en, 'en', RISING, 'the windows')).toEqual([
            en.drops.verdict.interval('+38%', '+12%', '+70%'),
            en.drops.verdict.rates('4.6', '3.3'),
            en.drops.verdict.events(64, 41),
            'the windows',
        ]);
    });

    it('should say why there is no verdict', () => {
        const details = verdictDetails(en, 'en', FEW, 'the windows');
        expect(details[0]).toBe('+100%');
        expect(details).toContain(en.drops.verdict.notEnough);
        expect(verdictDetails(en, 'en', STEADY, 'the windows')).toContain(en.drops.verdict.noClearChange);
    });
});

describe('trendWindows', () => {
    it('should compare today since midnight with the same hours of yesterday, in the server zone', () => {
        expect(trendWindows(en, 'en', SUMMARY_DAY)).toBe(en.drops.windows.today('00:00', '00:00', '20:00'));
    });

    it('should compare a week with the week before', () => {
        expect(trendWindows(en, 'en', SUMMARY)).toBe(en.drops.windows['7d']);
    });
});

/** A rising chip, its trigger and its tip. */
const chip = () => {
    render(
        <I18nProvider>
            <VerdictChip trend={RISING} windows="the windows" />
        </I18nProvider>,
    );
    const trigger = screen.getByRole('button', { name: 'Rising +38%' });
    const tip = screen.getByRole('tooltip', { hidden: true });
    return { trigger, tip };
};

describe('VerdictChip', () => {
    afterEach(cleanup);

    it('should carry its word and its icon, and be described by its details even closed', () => {
        const { trigger, tip } = chip();

        expect(trigger.querySelector('svg')).not.toBeNull();
        expect(trigger.getAttribute('aria-describedby')).toBe(tip.id);
        expect(trigger.getAttribute('aria-controls')).toBe(tip.id);
        expect(trigger.getAttribute('aria-expanded')).toBe('false');
        expect(tip.hidden).toBe(true);
        expect(trigger.contains(tip)).toBe(false);
        expect(tip.textContent).toContain(en.drops.verdict.rates('4.6', '3.3'));
    });

    it('should show on hover, stay while the pointer crosses into it, and go once it left both', async () => {
        const { trigger, tip } = chip();

        fireEvent.mouseEnter(trigger);
        expect(tip.hidden).toBe(false);
        expect(trigger.getAttribute('aria-expanded')).toBe('true');
        expect(screen.getByRole('tooltip').textContent).toContain(
            en.drops.verdict.interval('+38%', '+12%', '+70%'),
        );

        // On its way to the tip, which can be hovered, unlike a title.
        fireEvent.mouseLeave(trigger);
        fireEvent.mouseEnter(tip);
        await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
        expect(tip.hidden).toBe(false);

        fireEvent.mouseLeave(tip);
        await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
        expect(tip.hidden).toBe(true);
    });

    it('should stay open once clicked, and close on Escape or a second click', async () => {
        const { trigger, tip } = chip();

        fireEvent.click(trigger);
        fireEvent.mouseLeave(trigger);
        await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
        expect(tip.hidden).toBe(false);

        fireEvent.keyDown(window, { key: 'Escape' });
        expect(tip.hidden).toBe(true);

        fireEvent.click(trigger);
        expect(tip.hidden).toBe(false);
        fireEvent.click(trigger);
        expect(tip.hidden).toBe(true);
    });

    it('should close when something else is pressed', () => {
        const { trigger, tip } = chip();

        fireEvent.click(trigger);
        fireEvent.pointerDown(document.body);
        expect(tip.hidden).toBe(true);
    });
});
