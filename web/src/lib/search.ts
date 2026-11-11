// What the pages that keep their state in the address read it with. The address bar is the user's
// to edit: what does not read is left out rather than refused.

/** One of `choices`, or nothing. */
export const oneOf = <T extends string>(choices: readonly T[], value: unknown): T | undefined =>
    choices.find((choice) => choice === value);

/**
 * A text filter, trimmed; an empty one is left out. What the address parsed as a number or a
 * boolean, a licence's digits or a name such as `true`, comes back as the text it was.
 */
export const text = (value: unknown) => {
    const written = typeof value === 'number' || typeof value === 'boolean' ? String(value) : value;
    return typeof written === 'string' && written.trim() !== '' ? written.trim() : undefined;
};

/** Drops the fields that are not set, so that two equal searches read the same. */
export const defined = <T extends object>(object: T): T =>
    Object.fromEntries(Object.entries(object).filter(([, value]) => value !== undefined)) as T;
