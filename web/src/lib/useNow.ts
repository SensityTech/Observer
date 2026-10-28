import { useEffect, useState } from 'react';

/** The time, read again every `every` milliseconds, for uptimes and countdowns. */
export const useNow = (every = 1_000) => {
    const [now, setNow] = useState(Date.now);
    useEffect(() => {
        const timer = setInterval(() => setNow(Date.now()), every);
        return () => clearInterval(timer);
    }, [every]);
    return now;
};

/**
 * Seconds from `rfc3339` to `now`, for an uptime: whole minutes past the first, which the duration
 * format would otherwise round up. A clock behind the host's never reads as a time in the future.
 */
export const secondsSince = (rfc3339: string, now: number) => {
    const seconds = Math.floor(Math.max(0, now - Date.parse(rfc3339)) / 1000);
    return seconds < 60 ? seconds : seconds - (seconds % 60);
};
