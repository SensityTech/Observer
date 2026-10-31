import { TickMarkType, type Time, type UTCTimestamp } from 'lightweight-charts';

import { formatCalendarDay, formatClock, formatDay, zoneDate } from '../i18n/format';

// The drops page says "Days and times in <zone>": its chart reads every time in the schedule's zone,
// never the browser's, unlike the other charts of the panel.

const DAY = 86_400;

/** Buckets of a day are the days of the zone, which the chart keeps as calendar days. */
export const daily = (step: number) => step >= DAY;

/**
 * Where a bucket starting at `seconds` sits on the chart: its instant for buckets of an hour; for
 * buckets of a day, the zone's date, a calendar day the library never moves by the browser's zone.
 */
export const pointOf = (seconds: number, step: number, zone: string): Time =>
    daily(step) ? zoneDate(seconds * 1000, zone) : (seconds as UTCTimestamp);

/** The library's ticks, worded in the zone: `18:00` for a time, `28 Oct` for a day. */
export const tickWords = (locale: string, zone: string) => (time: Time, type: TickMarkType) => {
    if (typeof time === 'string') {
        return formatCalendarDay(locale, time, { weekday: false });
    }
    const instant = (time as number) * 1000;
    return type === TickMarkType.Time || type === TickMarkType.TimeWithSeconds
        ? formatClock(locale, instant, zone)
        : formatDay(locale, instant, zone, { weekday: false });
};

/** A chart time as a table or a tooltip writes it: `2026-10-29 21:00`, or `2026-10-29` for a day. */
export const stampOf = (time: Time, locale: string, zone: string) => {
    if (typeof time === 'string') {
        return time;
    }
    const instant = (time as number) * 1000;
    return `${zoneDate(instant, zone)} ${formatClock(locale, instant, zone)}`;
};

/** A bucket as a sentence says it: `29 Oct 21:00`, or `Thu 29 Oct` for a day. */
export const bucketWords = (seconds: number, step: number, locale: string, zone: string) => {
    if (daily(step)) {
        return formatCalendarDay(locale, zoneDate(seconds * 1000, zone));
    }
    return instantWords(seconds, locale, zone);
};

/** An instant as a sentence says it, a crash's or a restart's: `29 Oct 21:40`. */
export const instantWords = (seconds: number, locale: string, zone: string) =>
    `${formatDay(locale, seconds * 1000, zone, { weekday: false })} ${formatClock(locale, seconds * 1000, zone)}`;
