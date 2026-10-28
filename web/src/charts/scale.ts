const STEPS = [1, 2, 3, 5];

/**
 * The top of the players scale: the next 1, 2, 3 or 5 × 10ⁿ above the larger of the peak and the
 * slots, plus 10%, so that the slots line never touches the top (300 for 256).
 */
export const ceiling = (peak: number, slots: number | null) => {
    const wanted = Math.max(peak, slots ?? 0) * 1.1;
    let magnitude = 1;
    while (magnitude * 10 <= wanted) {
        magnitude *= 10;
    }
    for (;;) {
        for (const step of STEPS) {
            if (step * magnitude > wanted && step * magnitude >= 10) {
                return step * magnitude;
            }
        }
        magnitude *= 10;
    }
};
