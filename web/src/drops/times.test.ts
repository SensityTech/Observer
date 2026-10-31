import { TickMarkType, type UTCTimestamp } from 'lightweight-charts';
import { describe, expect, it } from 'vitest';

import { markerWords } from '../charts/MarkerOverlay';
import { en } from '../i18n/en';
import { bucketWords, instantWords, pointOf, stampOf, tickWords } from './times';

// 2026-10-30 20:00 UTC: 21:00 in Paris, 05:00 the next day in Tokyo, whatever this machine's zone.
const AT = 1_793_390_400;
const HOUR = 3_600;
const DAY = 86_400;

describe('the drops chart in the server zone', () => {
    it('should word its ticks in the zone, the hours and the days', () => {
        const tokyo = tickWords('en', 'Asia/Tokyo');
        expect(tokyo(AT as UTCTimestamp, TickMarkType.Time)).toBe('05:00');
        expect(tokyo(AT as UTCTimestamp, TickMarkType.DayOfMonth)).toBe('31 Oct');
        expect(tickWords('en', 'Europe/Paris')(AT as UTCTimestamp, TickMarkType.Time)).toBe('21:00');
        expect(tokyo('2026-10-31', TickMarkType.DayOfMonth)).toBe('31 Oct');
    });

    it("should keep a bucket of hours as its instant, and a bucket of a day as the zone's date", () => {
        expect(pointOf(AT, HOUR, 'Asia/Tokyo')).toBe(AT);
        // The day of Tokyo that starts at its midnight, 15:00 UTC the day before.
        expect(pointOf(AT - 5 * HOUR, DAY, 'Asia/Tokyo')).toBe('2026-10-31');
    });

    it('should stamp the crosshair and the tooltip in the zone', () => {
        expect(stampOf(AT as UTCTimestamp, 'en', 'Asia/Tokyo')).toBe('2026-10-31 05:00');
        expect(stampOf('2026-10-31', 'en', 'Asia/Tokyo')).toBe('2026-10-31');
    });

    it('should word a bucket and an instant in a sentence, never as an ISO date', () => {
        expect(bucketWords(AT, HOUR, 'en', 'Asia/Tokyo')).toBe('31 Oct 05:00');
        expect(bucketWords(AT - 5 * HOUR, DAY, 'en', 'Asia/Tokyo')).toBe('Sat 31 Oct');
        expect(instantWords(AT, 'en', 'Europe/Paris')).toBe('30 Oct 21:00');
    });

    it('should word the markers in the zone it is given', () => {
        const words = markerWords(en, [{ at: AT, kind: 'crash', backOnlineAt: AT + 120 }], HOUR, (seconds) =>
            instantWords(seconds, 'en', 'Asia/Tokyo'),
        );
        expect(words).toBe(
            en.charts.markers.inWords(en.charts.markers.crash, '31 Oct 05:00', '31 Oct 05:02'),
        );
    });
});
