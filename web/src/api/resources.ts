import { send } from './client';
import type { ResourceAck } from './generated/ResourceAck';
import type { ResourceOrder } from './generated/ResourceOrder';

const PATH = '/servers/default/resources';

/** What the Resources page asks of one resource, by FXServer's own command of that name. */
export type ResourceVerb = 'start' | 'stop' | 'restart';

/**
 * Has FXServer start, stop or restart a resource. The answer comes once its console has the line:
 * the `server.resources` topic says what became of it.
 */
export const orderResource = (verb: ResourceVerb, order: ResourceOrder) =>
    send<ResourceAck>('POST', `${PATH}/${verb}`, order);

/** Has FXServer read its resource folders again. */
export const rescanResources = () => send<ResourceAck>('POST', `${PATH}/rescan`);
