import { useEffect, useRef, type ReactNode } from 'react';

import { ditherField } from '../signin/dither';

/**
 * The sign-in's dotted grid and dither iris behind the server setup, out to the edges of the page
 * and in both themes. The iris moves as the sign-in's does, and stands still under reduced motion.
 */
export function SetupStage({ children }: { children: ReactNode }) {
    const field = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const host = field.current;
        return host === null ? undefined : ditherField(host);
    }, []);

    return (
        <div className="setup-stage">
            <div className="setup-backdrop" aria-hidden="true">
                <div className="setup-iris">
                    <div className="signin-dither" ref={field}>
                        <div className="signin-dither-still" />
                    </div>
                </div>
                <div className="setup-veil" />
            </div>
            {children}
        </div>
    );
}
