import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Trash2, TriangleAlert } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { allEntities, deleteEntities } from '../api/entities';
import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityLimits } from '../api/generated/EntityLimits';
import type { EntityResult } from '../api/generated/EntityResult';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { EntityMark, kindWord } from './marks';
import { breakdown, modelHash, scripted } from './model';

/**
 * Asks before deleting: what the selection is made of, what may go wrong, and from the number the
 * server sets, a word to type. The server checks each entity again; `onDone` gets what became of each.
 */
export function DeleteDialog({
    items,
    run,
    limits,
    onClose,
    onDone,
}: {
    items: readonly EntityItem[];
    /** The `run` of the list the entities were picked in. */
    run: string;
    limits: Pick<EntityLimits, 'typedConfirmationFrom'>;
    onClose: () => void;
    onDone: (results: EntityResult[]) => void;
}) {
    const { t } = useI18n();
    const words = t.entities.dialog;
    const client = useQueryClient();
    const [typed, setTyped] = useState('');
    const [only] = items;
    const single = items.length === 1 && only !== undefined ? only : null;
    const made = breakdown(items);
    const mustType = items.length >= limits.typedConfirmationFrom;
    const confirmed = !mustType || typed.trim() === words.word;
    const remove = useMutation({
        mutationFn: () =>
            deleteEntities({
                run,
                targets: items.map(({ netId, serial, model }) => ({ netId, serial, model })),
            }),
        onSuccess: async ({ results }) => {
            await client.invalidateQueries(allEntities);
            onDone(results);
        },
    });

    return (
        <Dialog
            role="alertdialog"
            title={single === null ? words.title(items.length) : words.titleOne(kindWord(t, single))}
            onClose={onClose}
            closeOnBackdrop={typed === ''}
            onSubmit={() => {
                if (confirmed && !remove.isPending) {
                    remove.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button
                        type="submit"
                        tone="danger"
                        icon={Trash2}
                        disabled={!confirmed}
                        pending={remove.isPending}
                    >
                        {single === null ? words.confirm(items.length) : words.confirmOne}
                    </Button>
                </>
            }
        >
            <p>{single === null ? words.body : words.bodyOne}</p>
            <ul className="flex flex-col overflow-hidden rounded-box border border-line bg-bg-subtle text-ui">
                {single === null ? (
                    <>
                        {made.groups.map((group) => (
                            <li
                                key={`${group.model}:${group.kind}:${group.resource ?? ''}`}
                                className="flex items-center gap-2.5 border-t border-line px-3 py-2 first:border-t-0"
                            >
                                <EntityMark kind={group.kind} />
                                <span className="font-mono text-small text-ink">
                                    {modelHash(group.model)}
                                </span>
                                <span className="min-w-0 flex-1 truncate text-ink-3">
                                    {group.resource ?? t.entities.noResource}
                                </span>
                                <span className="font-mono text-small text-ink-2">
                                    {words.times(group.count)}
                                </span>
                            </li>
                        ))}
                        {made.more > 0 && (
                            <li className="border-t border-line px-3 py-2 text-ink-3">
                                {words.more(made.more)}
                            </li>
                        )}
                    </>
                ) : (
                    <li className="flex items-center gap-2.5 px-3 py-2">
                        <EntityMark kind={single.kind} />
                        <span className="font-mono text-small text-ink">{modelHash(single.model)}</span>
                        <span className="min-w-0 flex-1 truncate text-ink-3">
                            {[
                                single.resource ?? t.entities.noResource,
                                t.entities.netId(single.netId),
                                t.entities.bucketOf(single.bucket),
                            ].join(' · ')}
                        </span>
                    </li>
                )}
            </ul>
            <Warnings>
                {single === null ? (
                    <>
                        {made.scripted > 0 && (
                            <Warning>
                                <b className="font-semibold">{words.scripted(made.scripted)}</b>{' '}
                                {words.scriptedWhy}
                            </Warning>
                        )}
                        {made.owned > 0 && (
                            <Warning>
                                <b className="font-semibold">{words.owned(made.owned)}</b>
                            </Warning>
                        )}
                    </>
                ) : (
                    <>
                        {single.resource !== null && (
                            <Warning>
                                <b className="font-semibold">{words.createdBy(single.resource)}</b>
                                {words.createdByWhy}
                            </Warning>
                        )}
                        {single.resource === null && scripted(single) && (
                            <Warning>
                                <b className="font-semibold">{words.scripted(1)}</b> {words.scriptedWhy}
                            </Warning>
                        )}
                        {single.owned && (
                            <Warning>
                                <b className="font-semibold">{words.ownedOne}</b>
                            </Warning>
                        )}
                    </>
                )}
            </Warnings>
            {mustType && (
                <TextField
                    label={words.type(words.word, items.length)}
                    value={typed}
                    spellCheck={false}
                    autoCapitalize="characters"
                    autoComplete="off"
                    inputClassName="font-mono text-ui"
                    onChange={(event) => setTyped(event.target.value)}
                />
            )}
            {remove.isError && <Notice tone="danger">{errorMessage(t, remove.error)}</Notice>}
        </Dialog>
    );
}

/** What may go wrong, gathered in one box; nothing when nothing may. */
function Warnings({ children }: { children: ReactNode }) {
    return (
        <div className="flex flex-col gap-1.5 rounded-box bg-warning-soft px-3.5 py-3 text-small text-warning empty:hidden">
            {children}
        </div>
    );
}

function Warning({ children }: { children: ReactNode }) {
    return (
        <p className="flex items-start gap-2.5">
            <TriangleAlert size={16} strokeWidth={1.5} className="mt-px flex-none" aria-hidden="true" />
            <span>{children}</span>
        </p>
    );
}
