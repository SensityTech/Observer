import type { Socket, SocketEvents } from '../live/realtime';
import { PROTOCOL } from '../live/realtime';
import { ApiError, send } from './client';
import type { WsTicket } from './generated/WsTicket';

/**
 * A ticket for the next socket. A refusal means the session cannot open one: it ended, or its user
 * must change their password first. The request itself already sent the panel where it belongs.
 */
export const requestTicket = async (): Promise<string | null> => {
    try {
        return (await send<WsTicket>('POST', '/auth/ws-ticket')).ticket;
    } catch (error) {
        if (error instanceof ApiError && (error.status === 401 || error.status === 403)) {
            return null;
        }
        throw error;
    }
};

/** The browser's websocket to this Observer, with the ticket in its URL: it cannot carry the session key. */
export const openSocket = (ticket: string, events: SocketEvents): Socket => {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${location.host}/api/ws?ticket=${encodeURIComponent(ticket)}`;
    const socket = new WebSocket(url, PROTOCOL);
    socket.addEventListener('message', (event) => {
        if (typeof event.data === 'string') {
            events.message(event.data);
        }
    });
    socket.addEventListener('close', (event) => events.close(event.code));
    return { send: (text) => socket.send(text), close: () => socket.close() };
};
