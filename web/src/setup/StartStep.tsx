import { Play, Power, RotateCw } from 'lucide-react';
import { useId, useState, type Ref } from 'react';

import type { CfgReport } from '../api/generated/CfgReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import { useArtifacts } from '../artifacts/context';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CheckRow } from '../components/CheckRow';
import { Dialog } from '../components/Dialog';
import { OneSyncChoice } from '../components/OneSyncChoice';
import { useI18n } from '../i18n/I18nProvider';
import { useServerStatus } from '../server/context';
import { FactRows } from './CfgStep';
import { Fact } from './Fact';
import { startAction } from './state';
import { StepActions } from './StepActions';

/** What follows the save: a start, a restart, or nothing. */
export type AfterSave = 'start' | 'restart' | null;

/**
 * How FXServer starts, a summary of what it starts from, then the save: followed by a start, or a
 * restart while FXServer runs, which asks first since players are disconnected.
 */
export function StartStep({
    titleRef,
    draft,
    report,
    saving,
    onChange,
    onSave,
    onBack,
}: {
    titleRef: Ref<HTMLHeadingElement>;
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
    const id = useId();
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
        <Card id={id} icon={Power} title={t.serverSetup.stepNames.start} titleRef={titleRef}>
            <div className="flex flex-col gap-5">
                <p className="max-w-2xl text-body text-ink-2">{words.lead}</p>
                <div className="flex max-w-2xl flex-col gap-5">
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
                </div>
                <section className="flex max-w-2xl flex-col gap-1.5">
                    <h3 className="text-ui font-medium text-ink">{words.summary}</h3>
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
        </Card>
    );
}
