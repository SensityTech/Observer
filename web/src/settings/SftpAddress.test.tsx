import { cleanup, fireEvent, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { FeatureView } from '../api/generated/FeatureView';
import type { Me } from '../api/generated/Me';
import type { SftpListen } from '../api/generated/SftpListen';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';
import { joinListen, splitListen } from './SftpAddress';

const words = en.settings.sftpAddress;

const LISTENING: SftpListen = {
    listen: '0.0.0.0:40126',
    pinned: false,
    open: true,
    fingerprint: 'SHA256:c29tZSBob3N0IGtleQ',
    refused: null,
};

/** Manages file access and views settings, without being the owner. */
const MANAGER: Me = {
    id: 2,
    username: 'grace',
    isOwner: false,
    permissions: 'fffff',
    mustChangePassword: false,
};

/** The File access page over an SFTP that listens, answering each change with `after`. */
const serve = (
    now: SftpListen,
    after: (listen: string | null) => SftpListen | Response = () => now,
    switches: Pick<FeatureView, 'feature' | 'enabled' | 'held'>[] = [],
) => {
    const sent: unknown[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${String(input).replace(/^\/api/, '')}`;
            if (route === 'GET /settings/sftp') {
                return Response.json(now);
            }
            if (route === 'PUT /settings/sftp') {
                const body = JSON.parse(String(init.body)) as { listen: string | null };
                sent.push(body);
                const reply = after(body.listen);
                return reply instanceof Response ? reply : Response.json(reply);
            }
            if (route === 'GET /features') {
                return Response.json({ features: switches });
            }
            if (route === 'GET /servers/default/files/groups') {
                return Response.json([]);
            }
            if (route === 'GET /files/accesses') {
                return Response.json({ users: [], services: [] });
            }
            return Response.json({ code: 'not_found', params: {} }, { status: 404 });
        }),
    );
    return sent;
};

describe('SFTP address', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should take an address apart and put it back, brackets around an IPv6 host', () => {
        expect(splitListen('0.0.0.0:40126')).toEqual({ host: '0.0.0.0', port: '40126' });
        expect(splitListen('[::1]:2022')).toEqual({ host: '::1', port: '2022' });
        expect(joinListen(' 10.0.0.4 ', ' 2022 ')).toBe('10.0.0.4:2022');
        expect(joinListen('::1', '2022')).toBe('[::1]:2022');
        expect(joinListen('', '2022')).toBeNull();
        expect(joinListen('0.0.0.0', '0')).toBeNull();
        expect(joinListen('0.0.0.0', '65536')).toBeNull();
        expect(joinListen('0.0.0.0', '22a')).toBeNull();
    });

    it('should show where SFTP listens with its host key, and move the port to this machine only', async () => {
        const sent = serve(LISTENING, (listen) => ({ ...LISTENING, listen }));
        const { getByText, getByRole, getByLabelText, findByText } = await openSettings('/settings/files');

        expect(getByText(words.listening('0.0.0.0:40126'))).not.toBeNull();
        // The host key whole, to compare, and a button that copies it.
        expect(getByText('SHA256:c29tZSBob3N0IGtleQ')).not.toBeNull();
        expect(getByRole('button', { name: words.copyKey }).title).toBe('SHA256:c29tZSBob3N0IGtleQ');
        // Nothing to move while the form says what is in force.
        expect((getByRole('button', { name: words.move }) as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(getByRole('button', { name: words.reaches.local }));
        fireEvent.change(getByLabelText(words.port), { target: { value: '2022' } });
        fireEvent.click(getByRole('button', { name: words.move }));

        expect(await findByText(words.listening('127.0.0.1:2022'))).not.toBeNull();
        expect(sent).toEqual([{ listen: '127.0.0.1:2022' }]);
    });

    it('should take one address of the host, refuse a port that is no port, and close the port', async () => {
        const sent = serve(LISTENING, (listen) =>
            listen === null ? { ...LISTENING, listen: null, open: false, fingerprint: null } : LISTENING,
        );
        const { getByRole, getByLabelText, findByText } = await openSettings('/settings/files');

        fireEvent.click(getByRole('button', { name: words.reaches.other }));
        const move = getByRole('button', { name: words.move }) as HTMLButtonElement;
        expect(move.disabled).toBe(true);
        fireEvent.change(getByLabelText(words.host), { target: { value: 'fd00::4' } });
        fireEvent.change(getByLabelText(words.port), { target: { value: '70000' } });
        expect(move.disabled).toBe(true);
        fireEvent.change(getByLabelText(words.port), { target: { value: '2022' } });
        fireEvent.click(move);
        await waitFor(() => expect(sent).toEqual([{ listen: '[fd00::4]:2022' }]));

        fireEvent.click(getByRole('button', { name: words.close }));

        expect(await findByText(words.none)).not.toBeNull();
        expect(sent[1]).toEqual({ listen: null });
        expect(getByRole('button', { name: words.open })).not.toBeNull();
    });

    it('should say why the port did not open in the words of the host, and try the same address again', async () => {
        const taken = 'Only one usage of each socket address is normally permitted. (os error 10048)';
        const refused: SftpListen = { ...LISTENING, open: false, fingerprint: null, refused: taken };
        const sent = serve(refused, () => LISTENING);
        const { getByText, getByRole, findByText } = await openSettings('/settings/files');

        expect(getByText(words.said(taken))).not.toBeNull();
        expect(getByText(words.refused('0.0.0.0:40126'), { exact: false })).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.again }));

        expect(await findByText(words.listening('0.0.0.0:40126'))).not.toBeNull();
        expect(sent).toEqual([{ listen: '0.0.0.0:40126' }]);
    });

    it('should leave an address observer.toml names to the host, and the form to the owner alone', async () => {
        serve({ ...LISTENING, pinned: true });
        const pinned = await openSettings('/settings/files');

        expect(pinned.getByText(words.listening('0.0.0.0:40126'))).not.toBeNull();
        expect(pinned.getByText(words.pinned)).not.toBeNull();
        expect(pinned.queryByLabelText(words.port)).toBeNull();
        expect(pinned.queryByRole('button', { name: words.close })).toBeNull();
        cleanup();

        serve(LISTENING);
        const manager = await openSettings('/settings/files', { me: MANAGER });
        expect(manager.getByText(words.listening('0.0.0.0:40126'))).not.toBeNull();
        expect(manager.queryByLabelText(words.port)).toBeNull();
        expect(manager.queryByRole('button', { name: words.close })).toBeNull();
    });

    it('should say what keeps a port closed that has its address: the switch, or observer.toml', async () => {
        const closed: SftpListen = { ...LISTENING, open: false, fingerprint: null };
        serve(closed, undefined, [{ feature: 'sftp', enabled: false, held: null }]);
        const off = await openSettings('/settings/files');
        expect(off.getByText(words.off('0.0.0.0:40126'))).not.toBeNull();
        cleanup();

        serve(closed, undefined, [{ feature: 'sftp', enabled: true, held: 'host' }]);
        const locked = await openSettings('/settings/files');
        expect(locked.getByText(words.locked('0.0.0.0:40126'))).not.toBeNull();
    });

    it('should say what the host answered when the address is refused', async () => {
        serve(LISTENING, () =>
            Response.json({ code: 'invalid_sftp_listen', params: { reason: 'address' } }, { status: 400 }),
        );
        const { getByRole, getByLabelText, findByText } = await openSettings('/settings/files');

        fireEvent.click(getByRole('button', { name: words.reaches.other }));
        fireEvent.change(getByLabelText(words.host), { target: { value: 'sftp.example.com' } });
        fireEvent.click(getByRole('button', { name: words.move }));

        expect(await findByText(en.errors.invalid_sftp_listen({ reason: 'address' }))).not.toBeNull();
    });
});
