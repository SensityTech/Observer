import { useMutation, useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Construction } from 'lucide-react';
import { useState } from 'react';

import { useAdmission } from '../admission/context';
import { Permission } from '../api/generated/Permission';
import { maintenancePreviewQuery, setMaintenance } from '../api/maintenance';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { MessagePreview } from '../components/MessagePreview';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { formatClock, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { fill } from '../lib/fill';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';
import { useSchedule } from '../schedule/context';

/**
 * Closes the server to players, from the status card, for an account that controls the server.
 * Gone while maintenance is on: the banner under the card ends it.
 */
export function MaintenanceButton() {
    const { t } = useI18n();
    const maintenance = useAdmission()?.maintenance;
    const [asking, setAsking] = useState(false);
    if (maintenance === undefined || maintenance.on) {
        return null;
    }
    return (
        <>
            <Button
                size="sm"
                tone="ghost"
                icon={Construction}
                className="controls-maintenance"
                onClick={() => setAsking(true)}
            >
                {t.maintenance.button}
            </Button>
            {asking && <MaintenanceDialog passes={maintenance.passes} onClose={() => setAsking(false)} />}
        </>
    );
}

/** Says what turning maintenance on does and what players will read, then asks. */
function MaintenanceDialog({ passes, onClose }: { passes: number; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.maintenance.dialog;
    const me = useMe();
    const preview = useQuery(maintenancePreviewQuery);
    const turnOn = useMutation({ mutationFn: () => setMaintenance(true), onSuccess: onClose });

    return (
        <Dialog
            role="alertdialog"
            title={words.title}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button
                        tone="ink"
                        icon={Construction}
                        pending={turnOn.isPending}
                        onClick={() => turnOn.mutate()}
                    >
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead(passes)}</p>
            {preview.isPending && (
                <div className="flex flex-col gap-2" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                </div>
            )}
            {preview.isError && (
                <Notice tone="warning" live={false}>
                    {words.previewFailed} {errorMessage(t, preview.error)}
                </Notice>
            )}
            {preview.isSuccess && (
                <MessagePreview
                    caption={words.preview}
                    message={fill(preview.data.message, t.settings.maintenance.example)}
                />
            )}
            {preview.data?.serverName != null && <p>{words.listed(preview.data.serverName)}</p>}
            <p className="text-ink-3">{words.connected}</p>
            {allows(me, Permission.SettingsView) && (
                <p>
                    <Link
                        to="/settings/$category"
                        params={{ category: 'maintenance' }}
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        onClick={onClose}
                    >
                        {words.change}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                </p>
            )}
            {turnOn.isError && <Notice tone="danger">{errorMessage(t, turnOn.error)}</Notice>}
        </Dialog>
    );
}

/**
 * Under the status card of every page while the server is closed for maintenance: since when, by
 * whom, how many still get in, and the way to end it for an account that controls the server.
 */
export function MaintenanceBanner() {
    const { t, locale } = useI18n();
    const me = useMe();
    const maintenance = useAdmission()?.maintenance;
    const zone = useSchedule().zone ?? 'UTC';
    const visit = useVisit(zone);
    const now = useNow(60_000);
    const end = useMutation({ mutationFn: () => setMaintenance(false) });
    if (maintenance === undefined || !maintenance.on) {
        return null;
    }
    const { since, by, passes } = maintenance;
    const when =
        since === null
            ? null
            : zoneDate(since, zone) === zoneDate(now, zone)
              ? formatClock(locale, since, zone)
              : visit(since);

    return (
        <div className="mt-3 flex flex-col gap-2 max-md:mt-2">
            <Notice
                tone="warning"
                icon={Construction}
                live={false}
                className="items-center max-md:items-start"
            >
                <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                    <p className="min-w-0 flex-1 basis-72">
                        <b className="font-semibold">{t.maintenance.on}</b>
                        {when !== null && by !== null && ` ${t.maintenance.since(when, by)}`}{' '}
                        {t.maintenance.refused(passes)}
                    </p>
                    {allows(me, Permission.ServerControl) && (
                        <Button
                            size="sm"
                            className="max-md:w-full"
                            pending={end.isPending}
                            onClick={() => end.mutate()}
                        >
                            {t.maintenance.end}
                        </Button>
                    )}
                </div>
            </Notice>
            {end.isError && (
                <Notice tone="danger">
                    {t.maintenance.endFailed} {errorMessage(t, end.error)}
                </Notice>
            )}
        </div>
    );
}
