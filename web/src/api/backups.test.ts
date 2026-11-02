import { describe, expect, it } from 'vitest';

import { copiesOf, downloadable, kindOf, pathStyleFor, placeOf, regionOf, tookSeconds } from './backups';
import type { BackupCopy } from './generated/BackupCopy';
import type { BackupRun } from './generated/BackupRun';

const copy = (id: number, kind: BackupCopy['kind'], state: BackupCopy['state']): BackupCopy => ({
    id,
    destinationId: id,
    destination: `Destination ${id}`,
    kind,
    state,
    location: `copy-${id}`,
    bytes: 412,
    startedAt: '2026-11-02T13:47:00Z',
    storedAt: state === 'stored' || state === 'deleted' ? '2026-11-02T13:47:30Z' : null,
    deletedAt: state === 'deleted' ? '2026-11-03T13:47:00Z' : null,
    deleteReason: state === 'deleted' ? 'retention' : null,
    error: state === 'failed' ? 'unreachable' : null,
    errorDetail: null,
});

const run = (copies: BackupCopy[]): BackupRun => ({
    id: 7,
    trigger: 'restart',
    actor: null,
    startedAt: '2026-11-02T13:47:00Z',
    finishedAt: '2026-11-02T13:47:38Z',
    outcome: 'partial',
    engine: 'mariaDb',
    database: 'es_extended',
    host: '127.0.0.1:3306',
    client: 'mariadb-dump 11.7.2-MariaDB',
    fileName: 'es_extended-20261102T134700Z-7.sql.gz',
    sqlBytes: 4_000_000,
    fileBytes: 412_000,
    fileSha256: 'a'.repeat(64),
    serverStarted: false,
    looseTables: [],
    looseTableCount: 0,
    error: null,
    errorDetail: null,
    copies,
});

describe('regionOf', () => {
    it('should read the region a known store names in its address', () => {
        expect(regionOf('https://s3.eu-central-003.backblazeb2.com')).toBe('eu-central-003');
        expect(regionOf('https://s3.eu-west-3.amazonaws.com')).toBe('eu-west-3');
        expect(regionOf('https://s3.fr-par.scw.cloud')).toBe('fr-par');
        expect(regionOf('https://0123abcd.r2.cloudflarestorage.com')).toBe('auto');
    });

    it("should fall back to AWS's default region for anything else", () => {
        expect(regionOf('https://s3.amazonaws.com')).toBe('us-east-1');
        expect(regionOf('http://127.0.0.1:9000')).toBe('us-east-1');
        expect(regionOf('')).toBe('us-east-1');
    });
});

describe('pathStyleFor', () => {
    it('should put the bucket in the host for AWS only', () => {
        expect(pathStyleFor('https://s3.eu-west-3.amazonaws.com')).toBe(false);
        expect(pathStyleFor('https://s3.eu-central-003.backblazeb2.com')).toBe(true);
        expect(pathStyleFor('http://127.0.0.1:9000')).toBe(true);
    });
});

describe('kindOf and placeOf', () => {
    it('should tell a share from a folder of this host', () => {
        expect(kindOf({ kind: 'local', folder: 'D:\\Backups\\sensity' })).toBe('local');
        expect(kindOf({ kind: 'local', folder: '\\\\nas\\backups' })).toBe('share');
        expect(kindOf({ kind: 'local', folder: '/mnt/nas/backups' })).toBe('share');
        expect(kindOf({ kind: 'local', folder: '/var/backups/fivem' })).toBe('local');
    });

    it('should say where each kind puts its copies on one line', () => {
        expect(
            placeOf({
                kind: 'sftp',
                host: 'u412.your-storagebox.de',
                port: 23,
                user: 'u412',
                folder: 'sensity',
                auth: 'key',
                hostKey: null,
            }),
        ).toBe('u412.your-storagebox.de:23/sensity');
        expect(
            placeOf({
                kind: 's3',
                endpoint: 'https://s3.eu-central-003.backblazeb2.com/',
                region: 'eu-central-003',
                bucket: 'sensity-backups',
                prefix: 'main/',
                accessKeyId: 'key',
                pathStyle: true,
            }),
        ).toBe('s3.eu-central-003.backblazeb2.com/sensity-backups/main/');
    });
});

describe('copies of a run', () => {
    it('should count copies that were stored, whether the retention let them go since', () => {
        expect(
            copiesOf(run([copy(1, 'local', 'stored'), copy(2, 's3', 'deleted'), copy(3, 'sftp', 'failed')])),
        ).toEqual({
            stored: 2,
            total: 3,
        });
    });

    it("should download a stored copy, this host's first", () => {
        const copies = [copy(1, 's3', 'stored'), copy(2, 'local', 'stored'), copy(3, 'local', 'deleted')];

        expect(downloadable(run(copies))?.id).toBe(2);
        expect(downloadable(run([copy(4, 'sftp', 'failed')]))).toBeNull();
    });

    it('should say how long a run took once it ended', () => {
        expect(tookSeconds(run([]))).toBe(38);
        expect(tookSeconds({ ...run([]), finishedAt: null })).toBeNull();
    });
});
