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

/** Seconds from `rfc3339` to `now`; a clock behind the host's never reads as a time in the future. */
export const secondsSince = (rfc3339: string, now: number) => Math.max(0, now - Date.parse(rfc3339)) / 1000;
