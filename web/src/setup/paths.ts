import type { FolderEntry } from '../api/generated/FolderEntry';
import type { FolderListing } from '../api/generated/FolderListing';

/**
 * The host's separator as a path shows it: a leading `/` says Linux, where a backslash is part of
 * a name; a drive letter or a backslash says Windows.
 */
export const separatorOf = (path: string) =>
    !path.startsWith('/') && (/^[A-Za-z]:/.test(path) || path.includes('\\')) ? '\\' : '/';

const isSeparator = (character: string, separator: string) =>
    character === separator || (separator === '\\' && character === '/');

/**
 * What a typed path asks to list: the folder up to its last separator, and the start of a name
 * after it. Null until a separator is typed, since `C:` or `srv` is no folder yet.
 */
export const typedFolder = (typed: string): { parent: string; prefix: string } | null => {
    const separator = separatorOf(typed);
    for (let at = typed.length - 1; at >= 0; at--) {
        if (isSeparator(typed[at] ?? '', separator)) {
            return { parent: typed.slice(0, at + 1), prefix: typed.slice(at + 1) };
        }
    }
    return null;
};

/** The path with its separator at the end, as a folder to go into. */
export const intoFolder = (path: string) => {
    const separator = separatorOf(path);
    return isSeparator(path.at(-1) ?? '', separator) ? path : `${path}${separator}`;
};

/** The folder's last name, a drive's letter included: `FXServer`, `D:`, `/`. */
export const folderName = (path: string) => {
    const separator = separatorOf(path);
    const trimmed = path.length > 1 && isSeparator(path.at(-1) ?? '', separator) ? path.slice(0, -1) : path;
    const parts = trimmed.split(separator === '\\' ? /[\\/]/ : '/');
    return parts.at(-1) || trimmed;
};

/** Each folder from the root down to `path`, with the path that lists it: `C:` is `C:\`. */
export const crumbs = (path: string): { name: string; path: string }[] => {
    const separator = separatorOf(path);
    const parts = path.split(separator === '\\' ? /[\\/]/ : '/').filter((part) => part !== '');
    if (separator === '/') {
        return [
            { name: '/', path: '/' },
            ...parts.map((name, index) => ({ name, path: `/${parts.slice(0, index + 1).join('/')}` })),
        ];
    }
    return parts.map((name, index) => ({
        name,
        path: index === 0 ? `${name}\\` : parts.slice(0, index + 1).join('\\'),
    }));
};

/** A folder the path field offers: a folder listed, or a server or txAdmin's data inside one. */
export type Suggestion = {
    entry: FolderEntry;
    /** The part the prefix matched, and the rest of the path after it. */
    head: string;
    tail: string;
    /** It sits inside the folder listed: `Sensity\FXServer`. */
    nested: boolean;
};

/**
 * What the field offers for a listing: each folder, with the servers and txAdmin's data inside it
 * right after it, the servers first. Unreadable folders stay, marked, but cannot be chosen.
 */
export const suggestionsOf = (listing: FolderListing): Suggestion[] => {
    const separator = separatorOf(listing.path);
    const all = listing.entries.flatMap((entry) => [
        { entry, head: entry.name, tail: '', nested: false },
        ...entry.inside.map((inside) => ({
            entry: inside,
            head: entry.name,
            tail: `${separator}${inside.name}`,
            nested: true,
        })),
    ]);
    const rank = (suggestion: Suggestion) => (suggestion.entry.kind === 'server' ? 0 : 1);
    return all.toSorted((a, b) => rank(a) - rank(b));
};

export const choosable = (entry: FolderEntry) => entry.kind !== 'unreadable';
