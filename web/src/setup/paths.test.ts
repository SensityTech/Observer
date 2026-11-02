import { describe, expect, it } from 'vitest';

import type { FolderEntry } from '../api/generated/FolderEntry';
import { crumbs, folderName, intoFolder, suggestionsOf, typedFolder } from './paths';

const folder = (name: string, path: string, extra: Partial<FolderEntry> = {}): FolderEntry => ({
    name,
    path,
    kind: 'folder',
    link: false,
    resources: null,
    cfg: null,
    folders: null,
    inside: [],
    ...extra,
});

describe('typedFolder', () => {
    it('should list the folder up to the last separator, with the start of a name after it', () => {
        expect(typedFolder('C:\\Users\\Ada\\Sen')).toEqual({ parent: 'C:\\Users\\Ada\\', prefix: 'Sen' });
        expect(typedFolder('C:/Users/')).toEqual({ parent: 'C:/Users/', prefix: '' });
        expect(typedFolder('/srv/fx')).toEqual({ parent: '/srv/', prefix: 'fx' });
        expect(typedFolder('/')).toEqual({ parent: '/', prefix: '' });
    });

    it('should wait for a separator before listing anything', () => {
        expect(typedFolder('C:')).toBeNull();
        expect(typedFolder('srv')).toBeNull();
        expect(typedFolder('')).toBeNull();
    });

    it('should take a backslash as part of a name on Linux', () => {
        expect(typedFolder('/srv/odd\\name')).toEqual({ parent: '/srv/', prefix: 'odd\\name' });
    });
});

describe('paths', () => {
    it('should add the separator a folder is gone into with, once', () => {
        expect(intoFolder('D:\\FiveM')).toBe('D:\\FiveM\\');
        expect(intoFolder('D:\\')).toBe('D:\\');
        expect(intoFolder('/srv')).toBe('/srv/');
    });

    it('should name a folder by its last part, a drive by its letter', () => {
        expect(folderName('C:\\Users\\Ada\\FXServer')).toBe('FXServer');
        expect(folderName('D:\\FiveM\\')).toBe('FiveM');
        expect(folderName('D:\\')).toBe('D:');
        expect(folderName('/srv/fx/live')).toBe('live');
        expect(folderName('/')).toBe('/');
    });

    it('should give each folder from the root down with the path that lists it', () => {
        expect(crumbs('C:\\Users\\Ada')).toEqual([
            { name: 'C:', path: 'C:\\' },
            { name: 'Users', path: 'C:\\Users' },
            { name: 'Ada', path: 'C:\\Users\\Ada' },
        ]);
        expect(crumbs('/srv/fx/')).toEqual([
            { name: '/', path: '/' },
            { name: 'srv', path: '/srv' },
            { name: 'fx', path: '/srv/fx' },
        ]);
    });
});

describe('suggestionsOf', () => {
    it('should offer the servers first, those inside a folder after its name, then the folders', () => {
        const server = folder('FXServer', 'D:\\Sensity\\FXServer', { kind: 'server', resources: 412 });
        const data = folder('txData', 'D:\\Sensity\\txData', { kind: 'txAdminData' });
        const listing = {
            path: 'D:\\',
            parent: null,
            problem: null,
            pathProblem: null,
            entries: [
                folder('Sensity', 'D:\\Sensity', { folders: 3, inside: [server, data] }),
                folder('Seed', 'D:\\Seed', { kind: 'unreadable' }),
            ],
            truncated: false,
        };

        const offered = suggestionsOf(listing).map((each) => `${each.head}${each.tail}`);

        expect(offered).toEqual(['Sensity\\FXServer', 'Sensity', 'Sensity\\txData', 'Seed']);
    });
});
