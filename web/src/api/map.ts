import { queryOptions } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

import { getJson, request, send } from './client';
import type { MapImage } from './generated/MapImage';
import type { MapImageRequest } from './generated/MapImageRequest';
import type { MapSettings } from './generated/MapSettings';
import type { MapView } from './generated/MapView';

const PATH = '/servers/default/players/map';

export const mapQuery = queryOptions({
    queryKey: ['map'],
    queryFn: ({ signal }) => getJson<MapView>(PATH, signal),
});

export const setMapImage = (path: string) =>
    send<MapImage>('PUT', `${PATH}/image`, { path } satisfies MapImageRequest);

export const removeMapImage = () => send('DELETE', `${PATH}/image`);

export const saveMapSettings = (settings: MapSettings) =>
    send<MapSettings>('PUT', `${PATH}/settings`, settings);

export const CADENCES = [1, 2, 5] as const;

/** The image of `version` as a `blob:` address: an `<img>` alone would not send the session key. */
export const fetchMapImage = async (version: string, signal?: AbortSignal) => {
    const response = await request('GET', `${PATH}/image?v=${encodeURIComponent(version)}`, { signal });
    return URL.createObjectURL(await response.blob());
};

/** The image as a `blob:` address while it is shown, fetched again when it changes and let go after. */
export const useMapImage = (version: string | null) => {
    const [image, setImage] = useState<{ version: string; url: string } | null>(null);
    useEffect(() => {
        if (version === null) {
            return;
        }
        const controller = new AbortController();
        let made: string | null = null;
        fetchMapImage(version, controller.signal)
            .then((url) => {
                made = url;
                setImage({ version, url });
            })
            .catch(() => {});
        return () => {
            controller.abort();
            if (made !== null) {
                URL.revokeObjectURL(made);
            }
        };
    }, [version]);
    return version !== null && image?.version === version ? image.url : null;
};
