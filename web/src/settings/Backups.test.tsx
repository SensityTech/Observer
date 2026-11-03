import { cleanup, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { BackupDestination } from '../api/generated/BackupDestination';
import type { BackupsView } from '../api/generated/BackupsView';
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const words = en.settings.backups;
const items = en.settings.items;

/** BACKUPS_VIEW alone. */
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '80000',
    mustChangePassword: false,
};

const destination = (id: number, name: string, enabled: boolean): BackupDestination => ({
    id,
    name,
    enabled,
    target: { kind: 'local', folder: 'D:\\Backups\\sensity' },
    retention: { keepLast: 7, keepDays: null },
    publicKey: null,
    hasSecret: false,
    version: 1,
    createdAt: '2026-11-01T10:00:00Z',
    updatedAt: '2026-11-01T10:00:00Z',
    stored: 1,
    storedBytes: 412_000,
    last: null,
});

const VIEW: BackupsView = {
    settings: {
        beforeRestarts: 'allRestarts',
        times: ['03:00', '15:00'],
        restartWaitSeconds: 300,
        engine: 'mariaDb',
        source: 'cfg',
        manual: null,
        clientFolder: null,
    },
    version: 3,
    hasPassword: false,
    zone: 'UTC',
    nextAt: null,
    nextRestartAt: null,
    runsLast30Days: 0,
    source: {
        kind: 'cfg',
        host: '127.0.0.1',
        port: 3306,
        socket: null,
        user: 'fivem',
        database: 'main',
        problem: null,
    },
    client: { path: null, flavour: null, version: null, problem: null },
    destinations: [destination(1, 'This machine', true), destination(2, 'Storage box', false)],
    running: null,
    recent: [],
};

const serve = (view: BackupsView | null) =>
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string) =>
            String(input) === '/api/servers/default/backups' && view !== null
                ? Response.json(view)
                : Response.json({ code: 'not_found', params: {} }, { status: 404 }),
        ),
    );

/** The value shown beside a setting of the category. */
const row = (id: keyof typeof items) => {
    const found = document.getElementById(`setting-${id}`);
    if (found === null) {
        throw new Error(`no row ${id}`);
    }
    return within(found);
};

describe('Backups settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should sum up the backups as the Backups page does, and lead to it', async () => {
        serve(VIEW);
        const { getByRole } = await openSettings('/settings/backups', { me: VIEWER });

        expect(row('backupsBeforeRestarts').getByText(en.backups.when.all)).not.toBeNull();
        expect(row('backupsTimes').getByText('03:00, 15:00')).not.toBeNull();
        expect(row('backupsDestinations').getByText(words.destination('Storage box', false))).not.toBeNull();
        expect(
            row('backupsLast').getByText(`${en.backups.summary.none} ${en.backups.summary.nothingPlanned}`),
        ).not.toBeNull();
        expect(getByRole('link', { name: new RegExp(words.openBackups) }).getAttribute('href')).toBe(
            '/backups',
        );
    });

    it('should say there is nowhere to send a backup yet', async () => {
        serve({ ...VIEW, destinations: [], settings: { ...VIEW.settings, times: [] } });
        await openSettings('/settings/backups', { me: VIEWER });

        expect(row('backupsDestinations').getByText(words.noDestination)).not.toBeNull();
        expect(row('backupsTimes').getByText(words.noTimes)).not.toBeNull();
    });

    it('should say why the backups could not be read', async () => {
        serve(null);
        const { findByText } = await openSettings('/settings/backups', { me: VIEWER });

        expect(await findByText(new RegExp(`^${en.backups.loadFailed}`))).not.toBeNull();
    });
});
