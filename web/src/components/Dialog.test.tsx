import { cleanup, render } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { Dialog } from './Dialog';

const dialog = (field: boolean) => (
    <Dialog title="Edit permissions" onClose={() => {}} footer={<button type="button">Cancel</button>}>
        {field ? <input aria-label="View the dashboard" type="checkbox" /> : <span>Loading</span>}
    </Dialog>
);

describe('Dialog', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(cleanup);

    it('should start on its first field', () => {
        const { getByLabelText } = render(dialog(true));

        expect(document.activeElement).toBe(getByLabelText('View the dashboard'));
    });

    it('should move to its first field once a loading body brings one', async () => {
        const { getByLabelText, rerender } = render(dialog(false));

        rerender(dialog(true));
        await Promise.resolve();

        expect(document.activeElement).toBe(getByLabelText('View the dashboard'));
    });

    it('should leave the focus where the user moved it while the body loaded', async () => {
        const { getByText, rerender } = render(dialog(false));
        getByText('Cancel').focus();

        rerender(dialog(true));
        await Promise.resolve();

        expect(document.activeElement).toBe(getByText('Cancel'));
    });
});
