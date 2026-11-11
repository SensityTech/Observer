/**
 * A message of the owner's as a player reads it: each `{key}` of `values` filled in, in one pass,
 * so that a value which holds `{name}` stays as written. Any other brace is left alone.
 */
export const fill = <Key extends string>(message: string, values: Record<Key, string>) =>
    message.replace(/\{(\w+)\}/g, (whole, key: string) =>
        Object.hasOwn(values, key) ? values[key as Key] : whole,
    );
