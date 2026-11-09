import { Play, RotateCw } from 'lucide-react';
import { useState, type ReactNode, type Ref } from 'react';

import type { CfgReport } from '../api/generated/CfgReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import { useArtifacts } from '../artifacts/context';
import { Button } from '../components/Button';
import { CheckRow } from '../components/CheckRow';
import { Dialog } from '../components/Dialog';
import { OneSyncChoice } from '../components/OneSyncChoice';
import { useI18n } from '../i18n/I18nProvider';
import { useServerStatus } from '../server/context';
import { FactRows } from './CfgStep';
import { Fact } from './Fact';
import { startAction } from './state';
import { StepActions } from './StepActions';
import { StepPage } from './StepPage';

/** What follows the save: a start, a restart, or nothing. */
export type AfterSave = 'start' | 'restart' | null;

/**
 * How FXServer starts, a summary of what it starts from, then the save: followed by a start, or a
 * restart while FXServer runs, which asks first since players are disconnected.
 */
export function StartStep({
    titleRef,
    banner,
    draft,
    report,
    saving,
    onChange,
    onSave,
    onBack,
}: {
    titleRef: Ref<HTMLHeadingElement>;
    banner?: ReactNode;
    draft: LaunchSettings;
    /** The cfg's last report, for the summary. */
    report: CfgReport | null;
    /** What follows the save under way; undefined while none is. */
    saving: AfterSave | undefined;
    onChange: (change: Partial<LaunchSettings>) => void;
    onSave: (after: AfterSave) => void;
    onBack: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.start;
    const { status: builds } = useArtifacts();
    const { status } = useServerStatus();
    const [confirming, setConfirming] = useState(false);
    const action = startAction(builds, status);
    const findings = report?.findings ?? [];
    const count = (severity: 'warning' | 'note') =>
        findings.filter((finding) => finding.severity === severity).length;
    const selected = builds?.selected ?? null;

    return (
        <StepPage
            step="start"
            titleRef={titleRef}
            title={t.serverSetup.headings.start}
            lead={words.lead}
            banner={banner}
            actions={
                <StepActions onBack={onBack} note={action === null ? words.withoutBuild : null}>
                    {action === null ? (
                        <Button tone="ink" pending={saving !== undefined} onClick={() => onSave(null)}>
                            {words.saveOnly}
                        </Button>
                    ) : (
                        <>
                            <Button
                                pending={saving === null}
                                disabled={saving !== undefined}
                                onClick={() => onSave(null)}
                            >
                                {words.saveOnly}
                            </Button>
                            <Button
                                tone="ink"
                                icon={action === 'restart' ? RotateCw : Play}
                                pending={saving === action}
                                disabled={saving !== undefined}
                                onClick={() => (action === 'restart' ? setConfirming(true) : onSave('start'))}
                            >
                                {action === 'restart' ? words.saveAndRestart : words.saveAndStart}
                            </Button>
                        </>
                    )}
                </StepActions>
            }
        >
            <div className="grid max-w-250 grid-cols-1 gap-4 xl:grid-cols-2">
                <section aria-label={words.how} className="card flex flex-col gap-5 p-5 max-md:p-4">
                    <OneSyncChoice value={draft.onesync} onChange={(onesync) => onChange({ onesync })} />
                    <div className="-mx-2 flex flex-col gap-1">
                        <CheckRow
                            label={t.server.autoStart}
                            description={t.server.autoStartHint}
                            checked={draft.autoStart}
                            onChange={(autoStart) => onChange({ autoStart })}
                        />
                        <CheckRow
                            label={t.server.txadminEvents}
                            description={t.server.txadminEventsHint}
                            checked={draft.txadminEvents}
                            onChange={(txadminEvents) => onChange({ txadminEvents })}
                        />
                    </div>
                </section>
                <section className="card flex flex-col gap-1.5 p-5 max-md:p-4">
                    <h2 className="text-ui font-medium text-ink">{words.summary}</h2>
                    <dl className="flex flex-col">
                        <Fact label={words.folder}>
                            <span className="font-mono text-ui">{draft.dataPath.trim()}</span>
                        </Fact>
                        <Fact label={words.cfg}>
                            <span className="font-mono text-ui">{draft.cfgFile.trim()}</span>
                        </Fact>
                        <Fact label={words.build}>
                            {selected === null ? (
                                words.noBuild
                            ) : (
                                <span className="font-mono text-ui">{selected}</span>
                            )}
                        </Fact>
                        {report?.facts != null && <FactRows facts={report.facts} />}
                        <Fact label={words.findings}>
                            {words.findingCount(count('warning'), count('note'))}
                        </Fact>
                    </dl>
                </section>
            </div>
            {confirming && (
                <Dialog
                    role="alertdialog"
                    title={t.controls.restartTitle}
                    onClose={() => setConfirming(false)}
                    footer={
                        <>
                            <Button tone="ghost" onClick={() => setConfirming(false)}>
                                {t.common.cancel}
                            </Button>
                            <Button
                                tone="ink"
                                icon={RotateCw}
                                onClick={() => {
                                    setConfirming(false);
                                    onSave('restart');
                                }}
                            >
                                {words.saveAndRestart}
                            </Button>
                        </>
                    }
                >
                    <p>{t.controls.connected(status?.players?.online ?? 0)}</p>
                    <p>{t.controls.noticeFirst}</p>
                </Dialog>
            )}
        </StepPage>
    );
}
