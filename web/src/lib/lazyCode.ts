import { lazy, type ComponentType } from 'react';

/**
 * A part of the panel whose code could not be loaded. The browser keeps that failure until the
 * page itself is loaded again, so trying again without a reload cannot work.
 */
export class CodeLoadError extends Error {
    constructor(cause: unknown) {
        super('a part of the panel could not be loaded', { cause });
        this.name = 'CodeLoadError';
    }
}

/** `lazy` for a component without props, with a failed load told apart from an error of it. */
export const lazyCode = (load: () => Promise<{ default: ComponentType }>) =>
    lazy(() =>
        load().catch((cause: unknown) => {
            throw new CodeLoadError(cause);
        }),
    );
