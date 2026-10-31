/// <reference types="vitest/jsdom" />
import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { PlainHttpNotice } from './PlainHttpNotice';

const WARNING = `${en.app.plainHttp.title} ${en.app.plainHttp.body}`;

/** The notice of a panel the browser reached at `url`. */
const reachedAt = (url: string) => {
    jsdom.reconfigure({ url });
    return render(
        <I18nProvider>
            <PlainHttpNotice />
        </I18nProvider>,
    );
};

describe('PlainHttpNotice', () => {
    afterEach(() => {
        cleanup();
        jsdom.reconfigure({ url: 'http://localhost:3000/' });
    });

    it('should warn when the panel is reached over plain HTTP at a LAN address', () => {
        const { getByRole } = reachedAt('http://192.168.1.20:40125/');

        expect(getByRole('alert').textContent).toBe(WARNING);
    });

    it('should warn when the panel is reached over plain HTTP at a host name', () => {
        const { getByRole } = reachedAt('http://observer.lan:40125/console');

        expect(getByRole('alert').textContent).toBe(WARNING);
    });

    it('should stay away when the panel is reached over HTTPS', () => {
        const { queryByRole } = reachedAt('https://panel.example.com/');

        expect(queryByRole('alert')).toBeNull();
    });

    it('should stay away on localhost', () => {
        const { queryByRole } = reachedAt('http://localhost:40125/');

        expect(queryByRole('alert')).toBeNull();
    });

    it('should stay away on a name under localhost', () => {
        const { queryByRole } = reachedAt('http://observer.localhost:40125/');

        expect(queryByRole('alert')).toBeNull();
    });

    it('should stay away on any address of 127.0.0.0/8', () => {
        for (const url of ['http://127.0.0.1:40125/', 'http://127.12.0.3:40125/']) {
            const { queryByRole, unmount } = reachedAt(url);

            expect(queryByRole('alert')).toBeNull();
            unmount();
        }
    });

    it('should stay away on the IPv6 loopback', () => {
        const { queryByRole } = reachedAt('http://[::1]:40125/');

        expect(queryByRole('alert')).toBeNull();
    });

    it('should warn on a name that only starts like a loopback address', () => {
        for (const url of ['http://127.0.0.1.example.com/', 'http://localhost.example.com/']) {
            const { getByRole, unmount } = reachedAt(url);

            expect(getByRole('alert').textContent).toBe(WARNING);
            unmount();
        }
    });
});
