import { queryOptions } from '@tanstack/react-query';

import { getJson, request, send } from './client';
import type { CommandAck } from './generated/CommandAck';
import type { LogFile } from './generated/LogFile';
import type { StartWarnings } from './generated/StartWarnings';

const SERVER = '/servers/default';

/** These answer once the supervisor took the request on; the `server.status` topic tells the rest. */
export const startServer = () => send('POST', `${SERVER}/start`);
export const stopServer = () => send('POST', `${SERVER}/stop`);
export const restartServer = () => send('POST', `${SERVER}/restart`);
/** Gives the start under way more time before Observer takes it for stuck. */
export const extendStart = () => send('POST', `${SERVER}/start/extend`);

/** What FXServer warned about as it last started its resources: the tools list it. */
export const startWarningsQuery = queryOptions({
    queryKey: ['start-warnings'],
    queryFn: ({ signal }) => getJson<StartWarnings>(`${SERVER}/start/warnings`, signal),
    staleTime: 10_000,
});

/** Audited before FXServer gets it; the answer says whether FXServer read it yet. */
export const sendCommand = (line: string) => send<CommandAck>('POST', `${SERVER}/console`, { line });

export const logFilesQuery = queryOptions({
    queryKey: ['console-logs'],
    queryFn: ({ signal }) => getJson<LogFile[]>(`${SERVER}/console/logs`, signal),
});

/** Hands one of the listed files to the browser, as the audit export does: a plain link carries no session key. */
export const downloadLog = async (name: string) => {
    const response = await request('GET', `${SERVER}/console/logs/${encodeURIComponent(name)}`);
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
