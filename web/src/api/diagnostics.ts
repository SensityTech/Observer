import { request, send } from './client';
import type { Crash } from './generated/Crash';

const CRASHES = '/servers/default/crashes';

/** The largest file the API hands out; a full dump stays on the host. */
export const MAX_DOWNLOAD = 64 * 1024 * 1024;

/** Hands one of a crash's files to the browser, as the console logs go: a plain link carries no session key. */
export const downloadCrashFile = async (crash: number, name: string) => {
    const response = await request('GET', `${CRASHES}/${crash}/files/${encodeURIComponent(name)}`);
    const url = URL.createObjectURL(await response.blob());
    try {
        const link = document.createElement('a');
        link.href = url;
        link.download = name;
        link.click();
    } finally {
        setTimeout(() => URL.revokeObjectURL(url), 0);
    }
};

/** Deletes a crash's files from the host; the record and its analysis stay. */
export const deleteCrashFiles = (crash: number) => send<Crash>('DELETE', `${CRASHES}/${crash}/files`);
