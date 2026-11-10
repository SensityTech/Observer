import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, History, Info, Save } from 'lucide-react';
import { useEffect, useId, useMemo, useState } from 'react';

import { cfgQuery, cfgReviewQuery, cfgVersionsQuery, saveCfg } from '../api/cfg';
import { ApiError } from '../api/client';
import type { CfgDocument } from '../api/generated/CfgDocument';
import type { CfgFinding } from '../api/generated/CfgFinding';
import type { CfgOutline } from '../api/generated/CfgOutline';
import type { CfgRefusal } from '../api/generated/CfgRefusal';
import type { Severity } from '../api/generated/Severity';
import { CfgEditor } from '../cfg/CfgEditor';
import { ChangesDialog, VersionsDialog } from '../cfg/dialogs';
import { GuidedForm } from '../cfg/GuidedForm';
import { UNTOUCHED, applyGuided, toggled, touched, type Guided } from '../cfg/guided';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { stringParam } from '../i18n/params';
import { useDebounced } from '../lib/useDebounced';
import { Findings } from '../setup/CfgStep';

/** How long the typing pauses before the text is read again. */
const REVIEW_AFTER_MS = 350;

const RANK = { blocking: 0, warning: 1, note: 2 } satisfies Record<Severity, number>;
const VIEWS = ['guided', 'file'] as const;
type View = (typeof VIEWS)[number];

/** What was done last, said above the fresh file. */
type Done = 'saved' | 'restored' | null;

/**
 * The launch's cfg, edited by field or as text. Its secrets never come to the browser: each is a
 * mark in the text, which a save puts back as it was unless something was typed over it.
 */
export function CfgPage() {
    const { t } = useI18n();
    const words = t.cfg;
    const document = useQuery(cfgQuery);
    const [done, setDone] = useState<Done>(null);

    if (document.isPending) {
        return (
            <>
                <div className="page-head">
                    <h1 className="page-title">{words.title}</h1>
                </div>
                <section className="card" aria-busy="true">
                    <div className="card-body flex flex-col gap-2">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/2" />
                        <span className="skeleton w-3/5" />
                    </div>
                </section>
            </>
        );
    }
    if (document.isError) {
        return <Unreadable error={document.error} onRetry={() => void document.refetch()} />;
    }
    // A save or a version put back gives another file: the edit starts over from it.
    return (
        <Editor
            key={document.data.base}
            document={document.data}
            done={done}
            onDone={setDone}
            reading={document.isFetching}
        />
    );
}

const REFUSALS = new Set<string>([
    'notConfigured',
    'missing',
    'notAFile',
    'unreadable',
    'readOnly',
    'tooLarge',
    'notUtf8',
    'holdsMarks',
] satisfies CfgRefusal[]);

/** Why the cfg is not edited from here, and the way out when there is one in the panel. */
function Unreadable({ error, onRetry }: { error: unknown; onRetry: () => void }) {
    const { t } = useI18n();
    const words = t.cfg;
    const reason =
        error instanceof ApiError && error.code === 'cfg_not_editable'
            ? stringParam(error.params, 'reason')
            : null;
    const known = reason !== null && REFUSALS.has(reason) ? (reason as CfgRefusal) : null;
    return (
        <>
            <div className="page-head">
                <h1 className="page-title">{words.title}</h1>
            </div>
            <Notice tone={known === null ? 'danger' : 'warning'}>
                <div className="flex flex-col items-start gap-2">
                    <p>{known === null ? errorMessage(t, error) : words.refused[known]}</p>
                    {known === 'notConfigured' ? (
                        <Link
                            to="/server/setup"
                            className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        >
                            {words.openSetup}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    ) : (
                        <Button size="sm" onClick={onRetry}>
                            {words.readAgain}
                        </Button>
                    )}
                </div>
            </Notice>
        </>
    );
}

/** The text the guided changes apply to, and its outline once it is known. */
type Edit = { text: string; outline: CfgOutline | null; guided: Guided };

function Editor({
    document,
    done,
    onDone,
    reading,
}: {
    document: CfgDocument;
    done: Done;
    onDone: (done: Done) => void;
    reading: boolean;
}) {
    const { t, format } = useI18n();
    const words = t.cfg;
    const client = useQueryClient();
    const checksId = useId();
    const [view, setView] = useState<View>('guided');
    const [edit, setEdit] = useState<Edit>({
        text: document.text,
        outline: document.check.outline,
        guided: UNTOUCHED,
    });
    const [jump, setJump] = useState<{ line: number; at: number } | null>(null);
    const [dialog, setDialog] = useState<'changes' | 'versions' | null>(null);

    const draft = useMemo(
        () =>
            edit.outline !== null && touched(edit.guided)
                ? applyGuided(edit.text, edit.outline, edit.guided)
                : edit.text,
        [edit],
    );
    const dirty = draft !== document.text;
    const settled = useDebounced(draft, REVIEW_AFTER_MS);
    const untouched = settled === document.text;
    const read = useQuery({
        ...cfgReviewQuery(settled),
        enabled: !untouched,
        placeholderData: keepPreviousData,
    });
    // The latest reading, of an earlier text while the next one is awaited.
    const review = untouched ? document.check : (read.data ?? document.check);
    const fresh = settled === draft && (untouched || (read.data !== undefined && !read.isPlaceholderData));
    // The outline of the text itself, not of what the guided changes make of it.
    const outline = edit.outline ?? (fresh && !touched(edit.guided) ? review.outline : null);

    useEffect(() => {
        if (!dirty) {
            return;
        }
        const warn = (event: BeforeUnloadEvent) => event.preventDefault();
        window.addEventListener('beforeunload', warn);
        return () => window.removeEventListener('beforeunload', warn);
    }, [dirty]);

    const accept = (next: CfgDocument, what: Done) => {
        client.setQueryData(cfgQuery.queryKey, next);
        void client.invalidateQueries({ queryKey: cfgVersionsQuery.queryKey });
        onDone(what);
    };
    const save = useMutation({
        mutationFn: () => saveCfg({ text: draft, base: document.base }),
        onMutate: () => onDone(null),
        onSuccess: (saved) => accept(saved, 'saved'),
    });
    const conflict = save.error instanceof ApiError && save.error.code === 'cfg_changed';

    const write = (text: string) => {
        onDone(null);
        setEdit({ text, outline: null, guided: UNTOUCHED });
    };
    const show = (next: View) => {
        // The File view edits the text itself: what the fields changed is written into it first.
        if (next === 'file' && touched(edit.guided)) {
            setEdit({ text: draft, outline: null, guided: UNTOUCHED });
        }
        setView(next);
    };
    const open = (finding: CfgFinding) => {
        if (finding.file === document.file && finding.line !== null) {
            show('file');
            setJump({ line: finding.line, at: Date.now() });
        }
    };
    const commentOut = (line: number) => {
        const lines = draft.split('\n');
        const held = lines[line - 1];
        if (held !== undefined) {
            lines[line - 1] = toggled(held, false);
            write(lines.join('\n'));
        }
    };

    const marks = new Map<number, Severity>();
    for (const finding of review.findings) {
        if (finding.file === document.file && finding.line !== null) {
            const held = marks.get(finding.line);
            if (held === undefined || RANK[finding.severity] < RANK[held]) {
                marks.set(finding.line, finding.severity);
            }
        }
    }
    const lines = document.text.split('\n').length - Number(document.text.endsWith('\n'));
    const started = document.check.outline.resources.filter((resource) => resource.enabled).length;
    const changed = Math.max(review.added, review.removed);

    return (
        <>
            <div className="page-head">
                <h1 className="page-title font-mono [font-variant-ligatures:none]">{document.file}</h1>
                <Segmented
                    label={words.viewLabel}
                    choices={VIEWS}
                    value={view}
                    words={words.views}
                    onChange={show}
                    className="ml-auto"
                />
            </div>
            <p className="-mt-1 text-small text-ink-3 tabular-nums">
                {[
                    words.meta(lines, started),
                    format.bytes(document.bytes),
                    ...(document.modifiedAt === null
                        ? []
                        : [words.modified(format.timestamp(document.modifiedAt))]),
                ].join(' · ')}
            </p>
            {done !== null && !dirty && <Notice tone="success">{words.done[done]}</Notice>}
            {save.isError && (
                <Notice tone="danger">
                    <div className="flex flex-col items-start gap-2">
                        <p>{conflict ? words.conflict : words.failed(errorMessage(t, save.error))}</p>
                        {conflict && (
                            <Button
                                size="sm"
                                pending={reading}
                                onClick={() => void client.invalidateQueries({ queryKey: cfgQuery.queryKey })}
                            >
                                {words.readAgain}
                            </Button>
                        )}
                    </div>
                </Notice>
            )}
            {view === 'guided' ? (
                <>
                    <p className="flex items-start gap-2.5 rounded-box border border-line bg-surface-2 px-3.5 py-3 text-small text-ink-2">
                        <Info size={16} strokeWidth={1.5} className="mt-px flex-none" aria-hidden="true" />
                        <span>
                            {words.intro} {document.secrets > 0 && words.secrets(document.secrets)}
                        </span>
                    </p>
                    {review.findings.length > 0 && (
                        <button
                            type="button"
                            className="inline-flex items-center gap-1.5 self-start text-small font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                            onClick={() => show('file')}
                        >
                            {words.checks.summary(review.findings.length)}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </button>
                    )}
                    {outline === null ? (
                        <section className="card" aria-busy="true">
                            <div className="card-body flex flex-col gap-2">
                                <span className="text-small text-ink-3">{words.reading}</span>
                                <span className="skeleton w-2/3" />
                                <span className="skeleton w-1/2" />
                            </div>
                        </section>
                    ) : (
                        <GuidedForm
                            outline={outline}
                            guided={edit.guided}
                            onChange={(guided) => {
                                onDone(null);
                                setEdit({ text: edit.text, outline, guided });
                            }}
                        />
                    )}
                </>
            ) : (
                <div className="grid grid-cols-1 items-start gap-3 xl:grid-cols-[minmax(0,1fr)_22rem]">
                    <CfgEditor
                        label={document.file}
                        text={draft}
                        marks={marks}
                        jump={jump}
                        onChange={write}
                    />
                    <aside className="card flex min-w-0 flex-col" aria-labelledby={checksId}>
                        <div className="card-head">
                            <h2 id={checksId} className="card-title">
                                {words.checks.title}
                            </h2>
                            <output className="text-small text-ink-3">
                                {fresh ? words.checks.hint : words.checks.checking}
                            </output>
                        </div>
                        <div className="card-body flex flex-col gap-4 max-md:p-4">
                            <Findings
                                findings={review.findings}
                                onOpen={open}
                                fix={{
                                    label: words.checks.commentOut,
                                    // Only a line read as the text stands, which holds nothing else.
                                    applies: (finding) =>
                                        fresh &&
                                        finding.kind === 'resourceMissing' &&
                                        finding.file === document.file &&
                                        review.outline.resources.some(
                                            (each) =>
                                                each.line === finding.line && each.enabled && each.alone,
                                        ),
                                    onFix: (finding) => {
                                        if (finding.line !== null) {
                                            commentOut(finding.line);
                                        }
                                    },
                                }}
                            />
                            {document.secrets > 0 && (
                                <p className="text-small text-ink-3">{words.secrets(document.secrets)}</p>
                            )}
                        </div>
                    </aside>
                </div>
            )}
            <div className="sticky bottom-3 z-20 mt-2 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-card border border-line bg-raise px-4 py-3 shadow-[var(--elev-md)] max-md:bottom-2">
                <output className="text-ui font-medium text-ink tabular-nums">
                    {!dirty ? words.bar.unchanged : fresh ? words.bar.changed(changed) : words.bar.pending}
                </output>
                <span className="min-w-0 flex-[1_1_16rem] text-small text-ink-3 max-md:hidden">
                    {words.bar.applies}
                </span>
                <div className="flex flex-wrap items-center gap-2">
                    <Button tone="ghost" icon={History} onClick={() => setDialog('versions')}>
                        {words.bar.versions(document.versions)}
                    </Button>
                    {dirty && (
                        <>
                            <Button tone="ghost" onClick={() => setDialog('changes')}>
                                {words.bar.changes}
                            </Button>
                            <Button
                                tone="ghost"
                                onClick={() => {
                                    save.reset();
                                    setEdit({
                                        text: document.text,
                                        outline: document.check.outline,
                                        guided: UNTOUCHED,
                                    });
                                }}
                            >
                                {words.bar.discard}
                            </Button>
                        </>
                    )}
                    <Button
                        tone="ink"
                        icon={Save}
                        disabled={!dirty}
                        pending={save.isPending}
                        onClick={() => save.mutate()}
                    >
                        {words.bar.save}
                    </Button>
                </div>
            </div>
            {dialog === 'changes' && <ChangesDialog text={draft} onClose={() => setDialog(null)} />}
            {dialog === 'versions' && (
                <VersionsDialog
                    document={document}
                    dirty={dirty}
                    onRestored={(restored) => {
                        setDialog(null);
                        accept(restored, 'restored');
                    }}
                    onClose={() => setDialog(null)}
                />
            )}
        </>
    );
}
