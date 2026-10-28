import { useEffect, useState } from 'react';

/** A wait of whole seconds that counts down every second, as the API's `retryAfter` asks. */
export const useCountdown = () => {
    const [until, setUntil] = useState<number | null>(null);
    const [now, setNow] = useState(0);

    useEffect(() => {
        if (until === null) {
            return;
        }
        const timer = window.setInterval(() => {
            const time = Date.now();
            setNow(time);
            if (time >= until) {
                setUntil(null);
            }
        }, 1000);
        return () => window.clearInterval(timer);
    }, [until]);

    const start = (seconds: number) => {
        const time = Date.now();
        setNow(time);
        setUntil(time + seconds * 1000);
    };

    return { seconds: until === null ? 0 : Math.max(0, Math.ceil((until - now) / 1000)), start };
};
