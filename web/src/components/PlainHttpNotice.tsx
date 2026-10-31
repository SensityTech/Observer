import { useI18n } from '../i18n/I18nProvider';
import { Notice } from './Notice';

// Nothing sent to a loopback address leaves the host, so nobody else can read it on the way. Browsers
// send every name under localhost there too, without asking DNS.
const LOOPBACK = /^(?:(?:.+\.)?localhost|127\.\d+\.\d+\.\d+|\[::1\])$/;

/** Shown, and never dismissed, while the panel is reached over plain HTTP at an address past loopback. */
export function PlainHttpNotice() {
    const { t } = useI18n();
    if (window.location.protocol !== 'http:' || LOOPBACK.test(window.location.hostname)) {
        return null;
    }
    return (
        <Notice tone="warning">
            <strong className="font-semibold">{t.app.plainHttp.title}</strong> {t.app.plainHttp.body}
        </Notice>
    );
}
