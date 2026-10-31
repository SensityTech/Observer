import { cleanup, fireEvent, render, screen } from '@testing-library/react';
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

describe('VerdictChip', () => {
    afterEach(cleanup);

    it('should carry its word and its icon, and show its details on focus', () => {
        render(
            <I18nProvider>
                <VerdictChip trend={RISING} windows="the windows" />
            </I18nProvider>,
        );

        const chip = screen.getByRole('button', { name: 'Rising +38%' });
        expect(chip.querySelector('svg')).not.toBeNull();
        const tooltip = screen.getByRole('tooltip', { hidden: true });
        expect(chip.getAttribute('aria-describedby')).toBe(tooltip.id);
        // Hidden, it still describes the chip, and it is never part of the chip's own words.
        expect(tooltip.hidden).toBe(true);
        expect(chip.contains(tooltip)).toBe(false);
        expect(tooltip.textContent).toContain(en.drops.verdict.rates('4.6', '3.3'));

        fireEvent.focus(chip);
        expect(tooltip.hidden).toBe(false);
        expect(screen.getByRole('tooltip').textContent).toContain(
            en.drops.verdict.interval('+38%', '+12%', '+70%'),
        );
        expect(tooltip.textContent).toContain('the windows');

        fireEvent.keyDown(window, { key: 'Escape' });
        expect(tooltip.hidden).toBe(true);
    });
});
