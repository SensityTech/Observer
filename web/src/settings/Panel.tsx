import { useQuery } from '@tanstack/react-query';
import { FileCog } from 'lucide-react';

import { observerSettingsQuery } from '../api/settings';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

/** Where the panel listens and whom it trusts, as `observer.toml` said at start: read here, changed there. */
export function PanelSettings() {
    const { t } = useI18n();
    const words = t.settings.panel;
    const settings = useQuery(observerSettingsQuery);
    const values = settings.data;

    return (
        <>
            <CategoryHead category="panel" />
            {settings.isError && (
                <Notice tone="danger">
                    {words.loadFailed} {errorMessage(t, settings.error)}
                </Notice>
            )}
            {settings.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {values !== undefined && (
                <SettingsGroup title={t.settings.categories.panel.name}>
                    <p className="flex items-start gap-2 border-b border-line bg-bg-subtle px-5 py-3 text-small text-ink-2 max-md:px-4">
                        <FileCog
                            size={14}
                            strokeWidth={1.5}
                            className="mt-0.5 flex-none text-ink-3"
                            aria-hidden="true"
                        />
                        <span className="min-w-0 [overflow-wrap:anywhere]">
                            {words.file(values.configFile)}
                        </span>
                    </p>
                    <SettingRow id="listen">
                        {() => <code className="font-mono text-ui text-ink">{values.listen}</code>}
                    </SettingRow>
                    <SettingRow id="hosts">
                        {() => <Values values={values.hosts} none={words.noHosts} />}
                    </SettingRow>
                    <SettingRow id="trustedProxies">
                        {() => <Values values={values.trustedProxies} none={words.noProxies} />}
                    </SettingRow>
                </SettingsGroup>
            )}
        </>
    );
}

function Values({ values, none }: { values: string[]; none: string }) {
    return values.length === 0 ? (
        <p className="text-body text-ink-3">{none}</p>
    ) : (
        <ul className="flex flex-col gap-1">
            {values.map((value) => (
                <li key={value} className="font-mono text-ui text-ink [overflow-wrap:anywhere]">
                    {value}
                </li>
            ))}
        </ul>
    );
}
