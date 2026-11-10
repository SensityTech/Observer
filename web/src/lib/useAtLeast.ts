import { useLayoutEffect, useState } from 'react';

/**
 * Whether an element is at least `width` pixels wide, followed as it resizes: the first value goes
 * on that element as its `ref`. A page's content sits between the rail and the players' column, so
 * its own width says what fits, which the window's does not. True where nothing measures.
 */
export const useAtLeast = (width: number) => {
    const [element, setElement] = useState<HTMLElement | null>(null);
    const [wide, setWide] = useState(true);
    useLayoutEffect(() => {
        if (element === null || typeof ResizeObserver === 'undefined') {
            return;
        }
        const measure = () => setWide(element.getBoundingClientRect().width >= width);
        measure();
        const resized = new ResizeObserver(measure);
        resized.observe(element);
        return () => resized.disconnect();
    }, [element, width]);
    return [setElement, wide] as const;
};
