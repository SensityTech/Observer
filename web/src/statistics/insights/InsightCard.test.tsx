import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { ApiError } from '../../api/client';
import { I18nProvider } from '../../i18n/I18nProvider';
import { CardState } from './InsightCard';

/** The state of a card that failed with `error`. */
const state = (error: unknown) =>
    render(
        <I18nProvider>
            <CardState pending={false} error={error} failed="The card could not be read." empty={null} />
        </I18nProvider>,
    );

describe('CardState', () => {
    afterEach(cleanup);

    it('should name the feature turned off and what holds it, without calling it a failure', () => {
        const off = new ApiError(404, 'feature_disabled', { feature: 'drops', held: 'requires' });

        const { container } = state(off);

        expect(container.textContent).toBe('Player drops needs another part of Observer that does not run.');
    });

    it('should prefix any other error with what failed', () => {
        const { container } = state(new ApiError(500, 'internal'));

        expect(container.textContent?.startsWith('The card could not be read. ')).toBe(true);
    });
});
