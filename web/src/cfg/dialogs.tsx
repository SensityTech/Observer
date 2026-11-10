import { useMutation, useQuery } from '@tanstack/react-query';
import { RotateCcw } from 'lucide-react';
import { useState } from 'react';

import { cfgDiffQuery, cfgVersionDiffQuery, cfgVersionsQuery, restoreCfg } from '../api/cfg';
import type { CfgDocument } from '../api/generated/CfgDocument';
import type { TextDiff } from '../api/generated/TextDiff';
import { CodeLines } from '../changes/CodeLines';
import { unifiedRows } from '../changes/diffRows';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

/** The lines of a diff, or what there is to say when it has none. */
function Changes({ diff, label }: { diff: TextDiff; label: string }) {
    const { t } = useI18n();
    const words = t.cfg.changes;
    if (diff.hunks.length === 0) {
        return <p className="text-small text-ink-3">{diff.lineEndingsOnly ? words.lineEnds : words.none}</p>;
    }
    return (
        <div className="flex min-w-0 flex-col gap-2">
            <p className="text-small text-ink-3 tabular-nums">{words.summary(diff.added, diff.removed)}</p>
            <div className="overflow-hidden rounded-box border border-line">
                <CodeLines rows={unifiedRows(diff.hunks)} numbering="both" wrap={false} label={label} />
            </div>
            {diff.pages > 1 && <p className="text-small text-ink-3">{words.more}</p>}
        </div>
    );
}

function Reading() {
    const { t } = useI18n();
    return (
        <div className="flex flex-col gap-2" aria-busy="true">
            <span className="sr-only">{t.common.loading}</span>
            <span className="skeleton w-2/3" />
            <span className="skeleton w-1/2" />
        </div>
    );
}

/** What a save of the text would change in the file, before it is saved. */
export function ChangesDialog({ text, onClose }: { text: string; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.cfg.changes;
    const diff = useQuery(cfgDiffQuery(text));
    return (
        <Dialog
            title={words.title}
            className="max-w-[60rem]"
            onClose={onClose}
            footer={<Button onClick={onClose}>{t.common.close}</Button>}
        >
            {diff.isPending && <Reading />}
            {diff.isError && <Notice tone="danger">{errorMessage(t, diff.error)}</Notice>}
            {diff.data !== undefined && <Changes diff={diff.data} label={words.lines} />}
        </Dialog>
    );
}

/**
 * The cfg as it was before each save: compared with the file, and put back, which keeps the file
 * as it is now as a version too.
 */
export function VersionsDialog({
    document,
    dirty,
    onRestored,
    onClose,
}: {
    document: CfgDocument;
    /** Changes not saved, which putting a version back would lose. */
    dirty: boolean;
    onRestored: (document: CfgDocument) => void;
    onClose: () => void;
}) {
    const { t, format } = useI18n();
    const words = t.cfg.versions;
    const versions = useQuery(cfgVersionsQuery);
    const [compared, setCompared] = useState<number | null>(null);
    const diff = useQuery({
        ...cfgVersionDiffQuery(compared ?? 0, document.base),
        enabled: compared !== null,
    });
    const restore = useMutation({
        mutationFn: (id: number) => restoreCfg(id, { base: document.base }),
        onSuccess: onRestored,
    });

    return (
        <Dialog
            title={words.title}
            className="max-w-[60rem]"
            onClose={onClose}
            footer={<Button onClick={onClose}>{t.common.close}</Button>}
        >
            <p className="text-small text-ink-3">{words.intro}</p>
            {dirty && <Notice tone="warning">{words.dirty}</Notice>}
            {restore.isError && <Notice tone="danger">{words.failed(errorMessage(t, restore.error))}</Notice>}
            {versions.isPending && <Reading />}
            {versions.isError && <Notice tone="danger">{errorMessage(t, versions.error)}</Notice>}
            {versions.data?.length === 0 && <p className="text-small text-ink-3">{words.none}</p>}
            {versions.data !== undefined && versions.data.length > 0 && (
                <ol className="flex flex-col">
                    {versions.data.map((version) => (
                        <li
                            key={version.id}
                            className="flex flex-col gap-3 border-b border-line py-3 last:border-b-0"
                        >
                            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                                <div className="flex min-w-0 flex-1 flex-col">
                                    <span className="text-ui font-medium text-ink tabular-nums">
                                        {format.timestamp(version.replacedAt, { seconds: true })}
                                    </span>
                                    <span className="text-small text-ink-3">
                                        {words.row(
                                            version.replacedBy,
                                            version.lines,
                                            format.bytes(version.bytes),
                                        )}
                                    </span>
                                </div>
                                <Button
                                    size="sm"
                                    tone="ghost"
                                    aria-expanded={compared === version.id}
                                    onClick={() => setCompared(compared === version.id ? null : version.id)}
                                >
                                    {compared === version.id ? words.hide : words.compare}
                                </Button>
                                <Button
                                    size="sm"
                                    icon={RotateCcw}
                                    disabled={dirty}
                                    pending={restore.isPending && restore.variables === version.id}
                                    onClick={() => restore.mutate(version.id)}
                                >
                                    {words.putBack}
                                </Button>
                            </div>
                            {compared === version.id && (
                                <>
                                    {diff.isPending && <Reading />}
                                    {diff.isError && (
                                        <Notice tone="danger">{errorMessage(t, diff.error)}</Notice>
                                    )}
                                    {diff.data !== undefined && (
                                        <Changes diff={diff.data} label={words.lines} />
                                    )}
                                </>
                            )}
                        </li>
                    ))}
                </ol>
            )}
        </Dialog>
    );
}
