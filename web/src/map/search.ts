// What the map's address may hold: the net id of an entity the list sends the map to.

/** The map's address; a field left out is the map as it opens. */
export type MapSearch = {
    /** An entity to show: its layer comes on and the map goes to it. */
    entity?: number;
};

const MAX_NET_ID = 65_535;

/** The address as the map reads it: what is not a net id is left out rather than refused. */
export const parseMapSearch = (raw: Record<string, unknown>): MapSearch => {
    const entity =
        typeof raw.entity === 'string' && /^\d+$/.test(raw.entity) ? Number(raw.entity) : raw.entity;
    return typeof entity === 'number' && Number.isSafeInteger(entity) && entity >= 1 && entity <= MAX_NET_ID
        ? { entity }
        : {};
};
