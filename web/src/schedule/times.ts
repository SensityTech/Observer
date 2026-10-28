// The "every X hours from HH:MM" row of the settings: it fills the list of times, and nothing of it
// is stored.

/** The steps that divide a day evenly. */
export const EVERY_HOURS = [1, 2, 3, 4, 6, 8, 12, 24] as const;

const MINUTES_A_DAY = 24 * 60;

/** The minute of the day of an `HH:MM` time, or null for anything else. */
export const minuteOf = (time: string) => {
    const match = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(time);
    return match === null ? null : Number(match[1]) * 60 + Number(match[2]);
};

export const timeOf = (minute: number) => {
    const wrapped = ((minute % MINUTES_A_DAY) + MINUTES_A_DAY) % MINUTES_A_DAY;
    return `${String(Math.floor(wrapped / 60)).padStart(2, '0')}:${String(wrapped % 60).padStart(2, '0')}`;
};

/** Ascending, as Observer stores them; a time it cannot read goes last. */
export const sortTimes = (times: string[]) =>
    times.toSorted((a, b) => (minuteOf(a) ?? MINUTES_A_DAY) - (minuteOf(b) ?? MINUTES_A_DAY));

/** Every `every` hours from `from`, around the clock: `8` from `22:30` is 06:30, 14:30, 22:30. */
export const fillTimes = (every: number, from: string): string[] => {
    const start = minuteOf(from);
    if (start === null || !EVERY_HOURS.some((hours) => hours === every)) {
        return [];
    }
    return sortTimes(Array.from({ length: 24 / every }, (_, index) => timeOf(start + index * every * 60)));
};
