import { useQuery } from '@tanstack/react-query';

import { ApiError } from '../api/client';
import { observerSettingsQuery } from '../api/settings';
import { useMe } from '../auth/me';
import { CopyField } from '../components/CopyField';
import { useI18n } from '../i18n/I18nProvider';

/** The section that gives SFTP its address, on the port Observer suggests beside the panel's. */
const SECTION = '[sftp]\nlisten = "0.0.0.0:40126"';

/** Whether a route of SFTP refused because `observer.toml` gives it no address. */
export const sftpNotSetUp = (error: unknown) =>
    error instanceof ApiError && error.code === 'feature_disabled' && error.params.held === 'notConfigured';

/**
 * What SFTP still needs while it is on without an address: the lines to add to `observer.toml`, the
 * file's place on the host for the owner, and the restart that opens the port.
 */
export function SftpSetup() {
    const { t } = useI18n();
    const words = t.settings.sftpSetup;
    const me = useMe();
    const observer = useQuery({ ...observerSettingsQuery, enabled: me.isOwner });
    const file = observer.data?.configFile ?? null;

    return (
        <div className="flex flex-col gap-3 rounded-box border border-line bg-surface-2 px-3.5 py-3">
            <p className="text-small font-medium text-ink">{words.title}</p>
            <ol className="flex list-decimal flex-col gap-2.5 pl-4.5 text-small text-ink-2 marker:text-ink-3">
                <li>
                    <span className="[overflow-wrap:anywhere]">
                        {file === null ? words.add : words.addTo(file)}
                    </span>
                    <div className="mt-2">
                        <CopyField label={words.lines} value={SECTION} multiline />
                    </div>
                </li>
                <li>{words.restart}</li>
                <li>{words.next}</li>
            </ol>
            <p className="text-meta text-ink-3">{words.why}</p>
        </div>
    );
}
