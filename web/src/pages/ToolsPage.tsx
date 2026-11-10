import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import {
    ArrowRight,
    Copy,
    FileCog,
    FileQuestion,
    Info,
    ScanSearch,
    Weight,
    type LucideIcon,
} from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';

import { cfgQuery } from '../api/cfg';
import type { ToolMode } from '../api/generated/ToolMode';
import type { ToolView } from '../api/generated/ToolView';
import { startWarningsQuery } from '../api/server';
import {
    applied,
    saveArchetypeSettings,
    scanArchetypes,
    toolsQuery,
    unstageArchetypes,
    withRun,
} from '../api/tools';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';

const MODES: ToolMode[] = ['off', 'report', 'fix'];
const FILTERS = ['all', 'attention', 'never'] as const;
type Filter = (typeof FILTERS)[number];

/** What a tool has to say: something to look at, nothing, or nothing yet. */
type Standing = 'attention' | 'clear' | 'never';

const LED = {
    attention: 'led-warning',
    clear: 'led-ok',
    never: 'led-off',
} satisfies Record<Standing, string>;

type Tool = {
    id: string;
    icon: LucideIcon;
    title: string;
    kind: string;
    about: string;
    standing: Standing;
    /** What it found, in a line. */
    found: string;
    /** When and from what. */
    source: string | null;
    actions: ReactNode;
};

const link =
    'inline-flex h-8 items-center gap-1.5 rounded-control border border-line bg-raise px-3 text-small font-medium text-ink shadow-[var(--elev-sm)] hover:border-line-strong max-md:h-11';

/**
 * Every tool of the server folder, each with what it found last and the way in: the checks
 * Observer runs itself, and what FXServer says as it starts that nobody reads in its console.
 */
export function ToolsPage() {
    const { t, format } = useI18n();
    const words = t.tools.hub;
    const client = useQueryClient();
    const { status } = useServerStatus();
    const tools = useQuery(toolsQuery);
    const warnings = useQuery(startWarningsQuery);
    const cfg = useQuery(cfgQuery);
    const [filter, setFilter] = useState<Filter>('all');
    const archetypes = tools.data?.tools.find((tool) => tool.tool === 'duplicateArchetypes') ?? null;
    const scan = useMutation({
        mutationFn: scanArchetypes,
        onSuccess: (run) => {
            client.setQueryData(toolsQuery.queryKey, (current) =>
                current === undefined ? current : withRun(current, run),
            );
            void client.invalidateQueries({ queryKey: toolsQuery.queryKey });
        },
    });

    const list: Tool[] = [];
    if (archetypes !== null) {
        const run = archetypes.lastRun?.report != null ? archetypes.lastRun : archetypes.lastReport;
        const counts = run?.report?.counts ?? null;
        // A fix run's report says what it wrote, no longer what there is to do.
        const written = run != null && applied(run, archetypes.open);
        const left = counts === null ? 0 : counts.groups - counts.groupsFixed;
        list.push({
            id: 'archetypes',
            icon: Copy,
            title: t.tools.archetypes.title,
            kind: words.kinds.assets,
            about: words.archetypes.about,
            standing:
                counts === null ? 'never' : (written ? left : counts.groups) > 0 ? 'attention' : 'clear',
            found:
                counts === null
                    ? words.neverScanned
                    : counts.groups === 0
                      ? words.archetypes.none
                      : written
                        ? words.archetypes.fixed(counts.groupsFixed, left)
                        : words.archetypes.found(counts.groups, counts.groupsFixed),
            source:
                run == null || counts === null
                    ? null
                    : words.archetypes.scanned(
                          format.timestamp(run.startedAt),
                          format.milliseconds(run.summary?.elapsedMs ?? 0),
                          counts.resources,
                      ),
            actions: (
                <>
                    <Link to="/tools/duplicate-archetypes" className={link}>
                        {counts !== null && counts.groups > 0 ? words.archetypes.review : words.open}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                    <Button
                        size="sm"
                        tone="ghost"
                        icon={ScanSearch}
                        pending={scan.isPending}
                        onClick={() => scan.mutate()}
                    >
                        {counts === null ? t.tools.archetypes.scan : words.archetypes.again}
                    </Button>
                </>
            ),
        });
    }
    const said = warnings.data ?? null;
    const from =
        said === null || said.startedAt === null ? null : words.fromStart(format.timestamp(said.startedAt));
    const heavy = said?.heavy ?? [];
    list.push({
        id: 'heavy',
        icon: Weight,
        title: words.heavy.title,
        kind: words.kinds.assets,
        about: words.heavy.about,
        standing: from === null ? 'never' : heavy.length > 0 ? 'attention' : 'clear',
        found:
            from === null
                ? words.noStart
                : heavy.length === 0
                  ? words.heavy.none
                  : words.heavy.found(heavy.length, heavy.filter((asset) => asset.oversized).length),
        source: from,
        actions: (
            <Link to="/tools/heavy-assets" className={link}>
                {words.list}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        ),
    });
    const findings = cfg.data?.check.findings ?? null;
    const unheld =
        findings?.filter(
            (finding) => finding.kind === 'resourceMissing' || finding.kind === 'resourceMissingMore',
        ).length ?? 0;
    list.push({
        id: 'cfg',
        icon: FileCog,
        title: words.cfg.title,
        kind: words.kinds.server,
        about: words.cfg.about,
        standing: findings === null ? 'never' : findings.length > 0 ? 'attention' : 'clear',
        found:
            findings === null
                ? cfg.isError
                    ? words.cfg.unread
                    : t.common.loading
                : findings.length === 0
                  ? words.cfg.none
                  : unheld === findings.length
                    ? words.cfg.unheld(unheld)
                    : words.cfg.found(findings.length),
        source: cfg.data == null ? null : words.cfg.source(cfg.data.file),
        actions: (
            <Link to="/cfg" className={link}>
                {words.cfg.open}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        ),
    });
    const missing = said?.missing ?? [];
    list.push({
        id: 'missing',
        icon: FileQuestion,
        title: words.missing.title,
        kind: words.kinds.resources,
        about: words.missing.about,
        standing: from === null ? 'never' : missing.length > 0 ? 'attention' : 'clear',
        found:
            from === null
                ? words.noStart
                : missing.length === 0
                  ? words.missing.none
                  : words.missing.found(missing.length, new Set(missing.map((file) => file.resource)).size),
        source: from,
        actions: (
            <Link to="/tools/missing-files" className={link}>
                {words.list}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        ),
    });

    const counts = {
        all: list.length,
        attention: list.filter((tool) => tool.standing === 'attention').length,
        never: list.filter((tool) => tool.standing === 'never').length,
    } satisfies Record<Filter, number>;
    const shown = list.filter(
        (tool) => filter === 'all' || tool.standing === (filter === 'attention' ? 'attention' : 'never'),
    );
    const running = status !== null && isRunning(status.state);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.tools.title} <span className="gloss">{t.tools.gloss}</span>
                </h1>
                <Segmented
                    label={words.show}
                    choices={FILTERS}
                    value={filter}
                    words={{
                        all: words.filters.all(counts.all),
                        attention: words.filters.attention(counts.attention),
                        never: words.filters.never(counts.never),
                    }}
                    onChange={setFilter}
                    className="ml-auto"
                />
            </div>
            <p className="-mt-2 max-w-3xl text-body text-ink-2">{words.lead}</p>
            {running && (
                <p className="flex items-start gap-2.5 rounded-box border border-line bg-surface-2 px-3.5 py-3 text-small text-ink-2">
                    <Info size={16} strokeWidth={1.5} className="mt-px flex-none" aria-hidden="true" />
                    <span>{words.running}</span>
                </p>
            )}
            {tools.isError && tools.data === undefined && (
                <Notice tone="danger">
                    {t.tools.loadFailed} {errorMessage(t, tools.error)}
                </Notice>
            )}
            {scan.isError && <Notice tone="danger">{errorMessage(t, scan.error)}</Notice>}
            <div className="grid grid-cols-1 items-start gap-3 xl:grid-cols-[minmax(0,1fr)_21rem]">
                <section aria-label={t.tools.title} className="grid grid-cols-1 gap-3 md:grid-cols-2">
                    {shown.map((tool) => (
                        <ToolCard key={tool.id} tool={tool} />
                    ))}
                    {shown.length === 0 && (
                        <p className="card p-5 text-small text-ink-3 md:col-span-2">{words.noneShown}</p>
                    )}
                </section>
                <aside className="flex min-w-0 flex-col gap-3">
                    <Staged view={archetypes} />
                    <BeforeStart view={archetypes} />
                    <LatestRuns view={archetypes} />
                </aside>
            </div>
        </div>
    );
}

function ToolCard({ tool }: { tool: Tool }) {
    const id = useId();
    const Icon = tool.icon;
    return (
        <article className="card flex min-w-0 flex-col" aria-labelledby={id}>
            <div className="flex flex-1 flex-col gap-3 p-5 max-md:p-4">
                <div className="flex items-center gap-2.5">
                    <span className="grid size-7 flex-none place-items-center rounded-[7px] border border-line bg-surface-2 text-ink-2">
                        <Icon size={15} strokeWidth={1.5} aria-hidden="true" />
                    </span>
                    <h2 id={id} className="min-w-0 flex-1 text-card text-ink [overflow-wrap:anywhere]">
                        {tool.title}
                    </h2>
                    <span className="eyebrow flex-none">{tool.kind}</span>
                </div>
                <p className="text-small text-ink-2">{tool.about}</p>
                <div className="mt-auto flex flex-col gap-1">
                    <p className="flex items-start gap-2 text-ui font-medium text-ink">
                        <span
                            className={cn('led mt-[7px] flex-none', LED[tool.standing])}
                            aria-hidden="true"
                        />
                        {tool.found}
                    </p>
                    {tool.source !== null && (
                        <p className="pl-[18px] text-meta text-ink-3 tabular-nums">{tool.source}</p>
                    )}
                </div>
            </div>
            <div className="flex flex-wrap items-center gap-2 border-t border-line px-5 py-3 max-md:px-4">
                {tool.actions}
            </div>
        </article>
    );
}

/** A side card of the hub. */
function Side({ title, children }: { title: string; children: ReactNode }) {
    const id = useId();
    return (
        <section className="card flex flex-col gap-3 p-5 max-md:p-4" aria-labelledby={id}>
            <h2 id={id} className="text-card text-ink">
                {title}
            </h2>
            {children}
        </section>
    );
}

/** The fix that waits for the next start, with the way to drop it. */
function Staged({ view }: { view: ToolView | null }) {
    const { t, format } = useI18n();
    const words = t.tools.hub.staged;
    const client = useQueryClient();
    const unstage = useMutation({
        mutationFn: unstageArchetypes,
        onSuccess: (tools) => client.setQueryData(toolsQuery.queryKey, tools),
    });
    const staged = view?.staged ?? null;
    return (
        <Side title={words.title}>
            {staged === null ? (
                <p className="text-small text-ink-3">{words.none}</p>
            ) : (
                <>
                    <p className="text-small text-ink-2">
                        {words.fix(
                            staged.fixes,
                            t.tools.archetypes.title,
                            staged.stagedBy,
                            format.timestamp(staged.stagedAt),
                        )}
                    </p>
                    <p className="text-small text-ink-3">{words.written}</p>
                    <Button
                        size="sm"
                        className="self-start"
                        pending={unstage.isPending}
                        onClick={() => unstage.mutate()}
                    >
                        {words.drop}
                    </Button>
                    {unstage.isError && <Notice tone="danger">{errorMessage(t, unstage.error)}</Notice>}
                </>
            )}
        </Side>
    );
}

/** What each tool does before FXServer starts, changed in place. */
function BeforeStart({ view }: { view: ToolView | null }) {
    const { t } = useI18n();
    const words = t.tools.hub.beforeStart;
    const client = useQueryClient();
    const save = useMutation({
        mutationFn: (atStart: ToolMode) => {
            if (view === null) {
                throw new Error('no tool to set');
            }
            return saveArchetypeSettings({
                settings: { ...view.settings, atStart },
                version: view.version,
            });
        },
        onSuccess: (tools) => client.setQueryData(toolsQuery.queryKey, tools),
    });
    return (
        <Side title={words.title}>
            {view !== null && (
                <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
                    <span className="text-ui text-ink">{t.tools.archetypes.title}</span>
                    <Segmented
                        label={words.label(t.tools.archetypes.title)}
                        choices={MODES}
                        value={
                            save.isPending && save.variables !== undefined
                                ? save.variables
                                : view.settings.atStart
                        }
                        words={words.modes}
                        onChange={(mode) => {
                            if (mode !== view.settings.atStart) {
                                save.mutate(mode);
                            }
                        }}
                    />
                </div>
            )}
            {save.isError && <Notice tone="danger">{errorMessage(t, save.error)}</Notice>}
            <p className="text-small text-ink-3">{words.hint}</p>
        </Side>
    );
}

/** The latest run of each tool, and the way to every one of them. */
function LatestRuns({ view }: { view: ToolView | null }) {
    const { t, format } = useI18n();
    const words = t.tools.hub.runs;
    const run = view?.lastRun ?? null;
    const archetypes = t.tools.archetypes;
    return (
        <Side title={words.title}>
            {run === null ? (
                <p className="text-small text-ink-3">{words.none}</p>
            ) : (
                <ul className="flex flex-col gap-2">
                    <li className="flex flex-col">
                        <span className="text-ui text-ink">
                            {words.run(archetypes.title, archetypes.modes[run.mode])}
                        </span>
                        <span className="text-meta text-ink-3 tabular-nums">
                            {[
                                format.timestamp(run.startedAt),
                                run.trigger === 'panel' && run.actor !== null
                                    ? archetypes.byActor(run.actor)
                                    : archetypes.triggers[run.trigger],
                                ...(run.outcome === null ? [] : [archetypes.outcomes[run.outcome]]),
                            ].join(' · ')}
                        </span>
                    </li>
                </ul>
            )}
            <Link
                to="/audit"
                className="inline-flex items-center gap-1.5 self-start text-small font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
            >
                {words.audit}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        </Side>
    );
}
