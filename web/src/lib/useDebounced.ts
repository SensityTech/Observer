import { useEffect, useState } from 'react';

/** `value` once it has stopped changing for `delay` milliseconds: what is typed, read when the typing pauses. */
export const useDebounced = <T>(value: T, delay: number) => {
    const [settled, setSettled] = useState(value);
    useEffect(() => {
        const timer = setTimeout(() => setSettled(value), delay);
        return () => clearTimeout(timer);
    }, [value, delay]);
    return settled;
};
