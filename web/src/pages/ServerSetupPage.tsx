import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { ApiError } from '../api/client';
import type { CfgCheck } from '../api/generated/CfgCheck';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderCheck } from '../api/generated/FolderCheck';
import type { FolderReport } from '../api/generated/FolderReport';
import type { LaunchField } from '../api/generated/LaunchField';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, cleaned, launchIssues, launchQuery } from '../api/launch';
import { restartServer, startServer } from '../api/server';
import { checkCfg, checkFolder, saveSetup } from '../api/serverSetup';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { Steps } from '../components/Steps';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useServerStatus } from '../server/context';
import { BuildStep } from '../setup/BuildStep';
import { CfgStep } from '../setup/CfgStep';
import { FirstLaunch } from '../setup/FirstLaunch';
import { FolderStep } from '../setup/FolderStep';
import { StartStep, type AfterSave } from '../setup/StartStep';
import {
    SETUP_STEPS,
    cfgAfterFolder,
    cfgCheckOf,
    preselectedOnesync,
    type Checked,
    type SetupStep,
} from '../setup/state';

/** The settings the setup keeps as saved, and does not show. */
const ARGUMENT_FIELDS = new Set<LaunchField>(['extraArguments', 'argumentName', 'argumentValue']);

type Outcome = {
    tone: 'success' | 'warning' | 'danger';
    message: string;
    startAgain?: boolean;
    toServer?: boolean;
};

/** Points Observer at an existing server: its folder, its cfg, a build, then the first start. */
export function ServerSetupPage() {
    const { t } = useI18n();
    const launch = useQuery(launchQuery);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.serverSetup.title} <span className="gloss">{t.serverSetup.gloss}</span>
                </h1>
            </div>
            {launch.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-3/4" />
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {launch.isError && (
                <Notice tone="danger">
                    {t.server.loadFailed} {errorMessage(t, launch.error)}
                </Notice>
            )}
            {launch.data !== undefined && <Setup saved={launch.data} />}
        </div>
    );
}

function Setup({ saved }: { saved: StoredLaunch | null }) {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const { status } = useServerStatus();
    // What the setup started from: the settings as stored then, and the version its save names.
    const [shown, setShown] = useState(saved);
    const [draft, setDraft] = useState<LaunchSettings>(saved?.settings ?? DEFAULT_LAUNCH);
    const [step, setStep] = useState<SetupStep>('folder');
    const [folder, setFolder] = useState<Checked<FolderCheck, FolderReport> | null>(null);
    const [cfg, setCfg] = useState<Checked<CfgCheck, CfgReport> | null>(null);
    // Until the owner picks one, the OneSync follows the cfg.
    const [onesyncPicked, setOnesyncPicked] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const [launched, setLaunched] = useState<{ before: ServerStatus | null } | null>(null);
    // The cfg a folder's detection put in the draft, which the next folder checked may replace.
    const taken = useRef<string | null>(null);
    const title = useRef<HTMLHeadingElement>(null);
    const moved = useRef(false);

    const go = (next: SetupStep) => {
        moved.current = true;
        setStep(next);
        setOutcome(null);
    };
    // The step's button the keyboard was on is gone with it: the keyboard goes to the new step's title.
    useEffect(() => {
        if (moved.current) {
            moved.current = false;
            title.current?.focus();
        }
    });

    const edit = (change: Partial<LaunchSettings>) => {
        setDraft((current) => ({ ...current, ...change }));
        setOutcome(null);
    };

    const folderCheck = useMutation({
        mutationFn: checkFolder,
        onSuccess: (report, request) => {
            setFolder({ request, report });
            if (report.problem === null) {
                const cfgFile = cfgAfterFolder(report, draft.cfgFile, taken.current);
                if (cfgFile !== draft.cfgFile) {
                    taken.current = cfgFile;
                }
                setDraft((current) => ({ ...current, cfgFile }));
            }
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });

    const cfgCheck = useMutation({
        mutationFn: checkCfg,
        onSuccess: (report, request) => {
            setCfg({ request, report });
            if (onesyncPicked) {
                return;
            }
            const onesync = preselectedOnesync(
                report.facts?.onesync ?? null,
                shown?.settings.onesync ?? null,
            );
            // How many players the cfg may allow depends on it: the report is made again with it.
            if (onesync !== request.onesync) {
                setDraft((current) => ({ ...current, onesync }));
                cfgCheck.mutate({ ...request, onesync });
            }
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const enterCfg = () => {
        go('cfg');
        cfgCheck.mutate(cfgCheckOf(draft));
    };

    const save = useMutation({
        mutationFn: async ({ after }: { after: AfterSave; before: ServerStatus | null }) => {
            const stored = await saveSetup({ settings: cleaned(draft), version: shown?.version ?? null });
            if (after === null) {
                return { stored, failure: null };
            }
            try {
                await (after === 'restart' ? restartServer() : startServer());
                return { stored, failure: null };
            } catch (failure) {
                return { stored, failure };
            }
        },
        onMutate: () => {
            setOutcome(null);
            setLaunched(null);
        },
        onSuccess: ({ stored, failure }, { after, before }) => {
            queryClient.setQueryData(launchQuery.queryKey, stored);
            setShown(stored);
            if (after === null) {
                setOutcome({ tone: 'success', message: t.server.saved });
            } else if (failure !== null) {
                setOutcome({
                    tone: 'danger',
                    message: t.serverSetup.start.startFailed(errorMessage(t, failure)),
                });
                // FXServer was tried and did not start: the first launch's own step says why.
                if (failure instanceof ApiError && failure.code === 'launch_failed') {
                    setLaunched({ before });
                }
            } else {
                setOutcome({ tone: 'success', message: t.serverSetup.start.starting });
                setLaunched({ before });
            }
        },
        onError: (error) => {
            if (error instanceof ApiError && error.code === 'conflict') {
                void queryClient.invalidateQueries({ queryKey: launchQuery.queryKey });
                setOutcome({ tone: 'warning', message: t.serverSetup.changedElsewhere, startAgain: true });
                return;
            }
            // The setup keeps the extra arguments as saved, with no field of theirs to say what is
            // wrong: the Server page has them.
            const issues =
                error instanceof ApiError && error.code === 'invalid_launch_settings'
                    ? launchIssues(error.params)
                    : [];
            if (issues.length > 0 && issues.every((issue) => ARGUMENT_FIELDS.has(issue.field))) {
                setOutcome({ tone: 'danger', message: t.serverSetup.start.argumentsRefused, toServer: true });
                return;
            }
            // The server checked again and found what blocks: its step checks again too, and says what.
            if (error instanceof ApiError && error.code === 'setup_blocked') {
                if (typeof error.params.folder === 'string') {
                    go('folder');
                    folderCheck.mutate({ dataPath: draft.dataPath.trim() });
                } else {
                    enterCfg();
                }
            }
            setOutcome({ tone: 'danger', message: errorMessage(t, error) });
        },
    });

    const startAgain = () => {
        setShown(saved);
        setDraft(saved?.settings ?? DEFAULT_LAUNCH);
        setFolder(null);
        setCfg(null);
        taken.current = null;
        setOnesyncPicked(false);
        setLaunched(null);
        go('folder');
    };

    const index = SETUP_STEPS.indexOf(step);
    const names = t.serverSetup.stepNames;
    const folderReport =
        folder !== null && folder.request.dataPath === draft.dataPath.trim() ? folder.report : null;

    return (
        <>
            <Steps
                label={t.serverSetup.steps}
                steps={SETUP_STEPS.map((key, at) => ({
                    key,
                    label: names[key],
                    passed: at < index,
                    failed: false,
                }))}
                current={index}
                busy={false}
                numbered
                said={t.serverSetup.said}
                compact={t.serverSetup.stepOf(index + 1, SETUP_STEPS.length, names[step])}
            />
            {outcome !== null && (
                <Notice tone={outcome.tone}>
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                        <span className="min-w-0 flex-1">{outcome.message}</span>
                        {outcome.startAgain && (
                            <Button size="sm" className="max-md:w-full" onClick={startAgain}>
                                {t.serverSetup.startAgain}
                            </Button>
                        )}
                        {outcome.toServer && (
                            <Link
                                to="/server"
                                className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                            >
                                {t.serverSetup.start.openServer}
                                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                            </Link>
                        )}
                    </div>
                </Notice>
            )}
            {step === 'folder' && (
                <FolderStep
                    titleRef={title}
                    draft={draft}
                    checked={folder}
                    checking={folderCheck.isPending}
                    onPath={(dataPath) => edit({ dataPath })}
                    onCheck={(dataPath) => folderCheck.mutate({ dataPath })}
                    onContinue={enterCfg}
                />
            )}
            {step === 'cfg' && (
                <CfgStep
                    titleRef={title}
                    draft={draft}
                    folder={folderReport}
                    checked={cfg}
                    checking={cfgCheck.isPending}
                    onCfg={(cfgFile) => edit({ cfgFile })}
                    onCheck={(cfgFile) => cfgCheck.mutate({ ...cfgCheckOf(draft), cfgFile })}
                    onBack={() => go('folder')}
                    onContinue={() => go('build')}
                />
            )}
            {step === 'build' && (
                <BuildStep titleRef={title} onBack={enterCfg} onContinue={() => go('start')} />
            )}
            {step === 'start' && (
                <>
                    <StartStep
                        titleRef={title}
                        draft={draft}
                        report={cfg?.report ?? null}
                        saving={save.isPending ? save.variables.after : undefined}
                        onChange={(change) => {
                            if (change.onesync !== undefined) {
                                setOnesyncPicked(true);
                                // The summary counts what the cfg allows with it, players included.
                                cfgCheck.mutate(cfgCheckOf({ ...draft, onesync: change.onesync }));
                            }
                            edit(change);
                        }}
                        onSave={(after) => save.mutate({ after, before: status })}
                        onBack={() => go('build')}
                    />
                    {launched !== null && <FirstLaunch before={launched.before} />}
                </>
            )}
        </>
    );
}
