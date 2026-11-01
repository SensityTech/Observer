import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Copy, ScanSearch, Wrench } from 'lucide-react';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import type { ServerState } from '../api/generated/ServerState';
import type { ToolRun } from '../api/generated/ToolRun';
import type { ToolView } from '../api/generated/ToolView';
import {
    applied,
    fixArchetypes,
    freshRun,
    scanArchetypes,
    scopeKey,
    toolsQuery,
    undoArchetypes,
    withRun,
} from '../api/tools';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { useServerStatus } from '../server/context';
import { FilesCard, type Outcome } from '../tools/FilesCard';
import { GroupsCard } from '../tools/GroupsCard';
import { SettingsCard } from '../tools/SettingsCard';

/** The states the tools write in, as Observer checks them: FXServer neither runs nor is about to. */
const STOPPED: ReadonlySet<ServerState> = new Set(['stopped', 'crashLoop']);

const OUTCOME_TONE = {
    done: 'success',
    cancelled: 'warning',
    timedOut: 'warning',
    failed: 'danger',
} as const;

type Count = keyof Messages['tools']['archetypes']['summary'];

/** The counts a run always shows, then those shown once they are not zero. */
const ALWAYS: Count[] = ['groups', 'fixable', 'left', 'openChanges'];
const WHEN_ANY: Count[] = ['fixed', 'filesChanged', 'removals', 'filesRestored'];

/** The server tools, the owner's: for now the duplicate archetypes tool alone. */
export function ToolsPage() {
    const { t } = useI18n();
    const tools = useQuery(toolsQuery);
    const archetypes = tools.data?.tools.find((tool) => tool.tool === 'duplicateArchetypes');

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.tools.title} <span className="gloss">{t.tools.gloss}</span>
                </h1>
            </div>
            {tools.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-3/4" />
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {/* A failed refetch keeps the data it had, and the settings typed in it. */}
            {tools.isError && tools.data === undefined && (
                <Notice tone="danger">
                    {t.tools.loadFailed} {errorMessage(t, tools.error)}
                </Notice>
            )}
            {archetypes !== undefined && <DuplicateArchetypes view={archetypes} />}
        </div>
    );
}

/**
 * The tool: what it does, its last run, Scan now and Fix, the groups of the latest report, the
 * files it changed and its settings. Fix and Undo run only while FXServer is stopped.
 */
function DuplicateArchetypes({ view }: { view: ToolView }) {
    const { t } = useI18n();
    const words = t.tools.archetypes;
    const id = useId();
    const queryClient = useQueryClient();
    const { status } = useServerStatus();
    const blocked =
        status === null ? t.tools.stateUnknown : STOPPED.has(status.state) ? null : t.tools.stopFirst;
    // A heal or an undo makes no report: the groups shown stay those of the latest one made.
    const reportRun = view.lastRun?.report != null ? view.lastRun : view.lastReport;
    const report = reportRun?.report ?? null;
    const plan = reportRun?.plan ?? null;
    const written = reportRun !== null && applied(reportRun);
    const fixable = report?.counts.groupsFixed ?? 0;
    const [notice, setNotice] = useState<Outcome | null>(null);
    const [undoNotice, setUndoNotice] = useState<Outcome | null>(null);
    const noticeRef = useRef<HTMLDivElement>(null);
    const undoNoticeRef = useRef<HTMLDivElement>(null);

    /** Shows the run at once, then reads the page again for the files it changed. */
    const took = (run: ToolRun) => {
        queryClient.setQueryData(toolsQuery.queryKey, (current) =>
            current === undefined ? current : withRun(current, run),
        );
        void queryClient.invalidateQueries({ queryKey: toolsQuery.queryKey });
    };
    /** What a run that ended early says, or null when it ran to its end. */
    const ended = (run: ToolRun): Outcome | null =>
        run.outcome === null || run.outcome === 'done'
            ? null
            : {
                  tone: OUTCOME_TONE[run.outcome],
                  message: words.ended(words.outcomes[run.outcome], run.summary?.problem ?? null),
              };

    const scan = useMutation({
        mutationFn: scanArchetypes,
        onMutate: () => setNotice(null),
        onSuccess: (run) => {
            took(run);
            setNotice(
                ended(run) ?? {
                    tone: 'success',
                    message: words.scanned(run.summary?.groups ?? 0, run.summary?.fixable ?? 0),
                },
            );
        },
        onError: (error) => setNotice({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const fix = useMutation({
        mutationFn: fixArchetypes,
        onMutate: () => setNotice(null),
        onSuccess: (run) => {
            took(run);
            setNotice(
                ended(run) ?? {
                    tone: 'success',
                    message: words.fixed(run.summary?.fixed ?? 0, run.summary?.filesChanged ?? 0),
                },
            );
        },
        onError: (error) => {
            // The plan changed since the report on screen: the fresh one takes its place, to review.
            const fresh =
                error instanceof ApiError && error.code === 'plan_changed' ? freshRun(error.params) : null;
            if (fresh !== null) {
                took(fresh);
            }
            setNotice({ tone: fresh === null ? 'danger' : 'warning', message: errorMessage(t, error) });
        },
    });
    const undo = useMutation({
        mutationFn: undoArchetypes,
        onMutate: () => setUndoNotice(null),
        onSuccess: (run) => {
            took(run);
            setUndoNotice(
                ended(run) ?? {
                    tone: 'success',
                    message: t.tools.changes.undone(run.summary?.filesRestored ?? 0),
                },
            );
        },
        onError: (error) => setUndoNotice({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const busy = scan.isPending || fix.isPending || undo.isPending;

    const fixReason =
        report === null || plan === null
            ? words.noReport
            : written
              ? words.applied
              : fixable === 0
                ? words.nothingToFix
                : blocked;

    // A button that goes away or turns disabled leaves the keyboard on the body: it moves to the
    // notice saying what happened.
    useEffect(() => {
        if (notice !== null && document.activeElement === document.body) {
            noticeRef.current?.focus();
        }
    }, [notice]);
    useEffect(() => {
        if (undoNotice !== null && document.activeElement === document.body) {
            undoNoticeRef.current?.focus();
        }
    }, [undoNotice]);

    return (
        <>
            <Card
                id={`${id}-tool`}
                icon={Copy}
                title={words.title}
                aside={
                    <>
                        <Button
                            size="sm"
                            icon={ScanSearch}
                            pending={scan.isPending}
                            disabled={busy}
                            onClick={() => scan.mutate()}
                        >
                            {words.scan}
                        </Button>
                        <Button
                            size="sm"
                            tone="ink"
                            icon={Wrench}
                            pending={fix.isPending}
                            disabled={busy || fixReason !== null}
                            aria-describedby={`${id}-fix`}
                            onClick={() => {
                                if (plan !== null) {
                                    fix.mutate(plan);
                                }
                            }}
                        >
                            {words.fix}
                        </Button>
                    </>
                }
            >
                <div className="flex max-w-3xl min-w-0 flex-col gap-5">
                    <div className="flex flex-col gap-2">
                        <p className="text-body text-ink-2">{words.about}</p>
                        <p id={`${id}-fix`} className="text-small text-ink-3">
                            {fixReason ??
                                (report === null
                                    ? null
                                    : words.fixPlan(
                                          fixable,
                                          report.counts.removals,
                                          report.counts.filesChanged,
                                      ))}
                        </p>
                    </div>
                    {notice !== null && (
                        <Notice ref={noticeRef} tabIndex={-1} tone={notice.tone}>
                            {notice.message}
                        </Notice>
                    )}
                    <LastRun run={view.lastRun} earlier={reportRun !== view.lastRun ? reportRun : null} />
                </div>
            </Card>
            <GroupsCard report={report} applied={written} />
            <FilesCard
                view={view}
                blocked={blocked}
                busy={busy}
                pending={undo.isPending && undo.variables !== undefined ? scopeKey(undo.variables) : null}
                notice={undoNotice}
                noticeRef={undoNoticeRef}
                onUndo={(scope) => undo.mutate(scope)}
            />
            <SettingsCard view={view} />
        </>
    );
}

/** When the latest run ran, who asked, what it did and how it ended, and its counts. */
function LastRun({ run, earlier }: { run: ToolRun | null; earlier: ToolRun | null }) {
    const { t, format } = useI18n();
    const words = t.tools.archetypes;
    const id = useId();
    const summary = run?.summary ?? null;
    const counts = summary === null ? [] : [...ALWAYS, ...WHEN_ANY.filter((key) => summary[key] !== 0)];

    return (
        <section aria-labelledby={id} className="flex flex-col gap-3">
            <h3 id={id} className="text-ui font-semibold text-ink">
                {words.lastRun}
            </h3>
            {run === null ? (
                <p className="text-ui text-ink-3">{words.noRun}</p>
            ) : (
                <>
                    <dl className="grid grid-cols-2 gap-x-6 gap-y-3 max-md:grid-cols-1 lg:grid-cols-4">
                        <Fact term={words.when}>
                            <span className="font-mono text-ui">
                                {run.finishedAt === null || summary === null
                                    ? format.timestamp(run.startedAt)
                                    : words.took(
                                          format.timestamp(run.startedAt),
                                          format.milliseconds(summary.elapsedMs),
                                      )}
                            </span>
                        </Fact>
                        <Fact term={words.by}>
                            {run.trigger === 'panel' && run.actor !== null
                                ? words.byActor(run.actor)
                                : words.triggers[run.trigger]}
                        </Fact>
                        <Fact term={words.mode}>{words.modes[run.mode]}</Fact>
                        <Fact term={words.outcome}>
                            {run.outcome === null ? (
                                <Chip>{words.running}</Chip>
                            ) : (
                                <Chip tone={OUTCOME_TONE[run.outcome]}>{words.outcomes[run.outcome]}</Chip>
                            )}
                            {summary?.problem != null && (
                                <span className="mt-1 block text-small text-ink-2 [overflow-wrap:anywhere]">
                                    {summary.problem}
                                </span>
                            )}
                        </Fact>
                    </dl>
                    {summary !== null && (
                        <dl className="flex flex-wrap gap-x-7 gap-y-2">
                            {counts.map((key) => (
                                <div key={key} className="flex flex-col">
                                    <dt className="text-meta text-ink-3">{words.summary[key]}</dt>
                                    <dd className="text-figure-sm text-ink">{format.number(summary[key])}</dd>
                                </div>
                            ))}
                        </dl>
                    )}
                </>
            )}
            {earlier !== null && (
                <p className="text-small text-ink-3">
                    {words.reportFrom(format.timestamp(earlier.startedAt))}
                </p>
            )}
        </section>
    );
}

function Fact({ term, children }: { term: string; children: ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="text-body text-ink">{children}</dd>
        </div>
    );
}
