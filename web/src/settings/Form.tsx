import {
    createContext,
    use,
    useId,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
    type RefObject,
} from 'react';

import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { APPLIES, type Applies, type SettingId } from './catalog';

/** The order the save bar names when changes apply in, soonest first. */
const SOONEST: Applies[] = ['now', 'withinHour', 'nextRun', 'nextStart', 'observerStart', 'cleanup', 'file'];

type Actions = { save: () => void; discard: () => void };

/** What a section of a category tells the save bar: what changed in it, and how to save or drop that. */
type Entry = {
    changed: readonly SettingId[];
    pending: boolean;
    /** Nothing stored yet: what is shown is to be saved as it is, and there is nothing to go back to. */
    unsaved: boolean;
    actions: RefObject<Actions>;
};

type Sections = {
    put: (key: string, entry: Entry) => void;
    drop: (key: string) => void;
};

const SectionsContext = createContext<Sections | null>(null);

/**
 * A category's settings: one form whose sections each check and save their own, through one save
 * bar that counts every change of the category and says when they apply.
 */
export function SettingsForm({ children }: { children: ReactNode }) {
    const [entries, setEntries] = useState<ReadonlyMap<string, Entry>>(() => new Map());
    const sections = useMemo<Sections>(
        () => ({
            put: (key, entry) => setEntries((current) => new Map(current).set(key, entry)),
            drop: (key) =>
                setEntries((current) => {
                    const next = new Map(current);
                    next.delete(key);
                    return next;
                }),
        }),
        [],
    );
    const waiting = [...entries.values()].filter((entry) => entry.changed.length > 0 || entry.unsaved);

    return (
        <SectionsContext value={sections}>
            <form
                className="flex flex-col gap-4"
                noValidate
                onSubmit={(event) => {
                    event.preventDefault();
                    for (const entry of waiting) {
                        if (!entry.pending) {
                            entry.actions.current.save();
                        }
                    }
                }}
            >
                {children}
                <SaveBar
                    waiting={waiting}
                    onDiscard={() => {
                        for (const entry of waiting) {
                            entry.actions.current.discard();
                        }
                    }}
                />
            </form>
        </SectionsContext>
    );
}

/**
 * Whether the keyboard is nowhere, or on the save bar, which goes once nothing is left to save: a
 * section then moves it to the notice saying what its save or discard did.
 */
export const keyboardLeft = () => {
    const active = document.activeElement;
    return active === null || active === document.body || active.closest('[data-save-bar]') !== null;
};

/** Joins a section to the category's save bar for as long as it is shown. */
export function useSection({
    changed,
    pending,
    unsaved = false,
    save,
    discard,
}: {
    changed: readonly SettingId[];
    pending: boolean;
    unsaved?: boolean;
    save: () => void;
    discard: () => void;
}) {
    const sections = use(SectionsContext);
    const key = useId();
    const actions = useRef<Actions>({ save, discard });
    // The bar calls the latest of them, while the entry only changes with what the bar shows.
    useLayoutEffect(() => {
        actions.current = { save, discard };
    });
    const signature = changed.join(' ');
    useLayoutEffect(() => {
        const ids = signature === '' ? [] : (signature.split(' ') as SettingId[]);
        sections?.put(key, { changed: ids, pending, unsaved, actions });
    }, [sections, key, signature, pending, unsaved]);
    useLayoutEffect(() => () => sections?.drop(key), [sections, key]);
}

function SaveBar({ waiting, onDiscard }: { waiting: Entry[]; onDiscard: () => void }) {
    const { t } = useI18n();
    const words = t.settings.bar;
    if (waiting.length === 0) {
        return null;
    }
    const changed = waiting.flatMap((entry) => entry.changed);
    const unsaved = waiting.some((entry) => entry.unsaved);
    const applies = new Set(changed.map((id) => APPLIES.get(id)));
    const when = SOONEST.filter((each) => applies.has(each)).map((each) => t.settings.appliesWhen[each]);

    return (
        <div
            data-save-bar
            className="sticky bottom-3 z-(--z-dock) flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop"
        >
            <p className="mr-auto flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-ui">
                <span className="led led-warning" aria-hidden="true" />
                <output className="text-ink">
                    {unsaved && changed.length === 0 ? words.notSaved : words.changes(changed.length)}
                </output>
                {when.length > 0 && <span className="text-ink-3">· {words.applies(when)}</span>}
            </p>
            {!unsaved && (
                <Button tone="ghost" className="max-md:flex-1" onClick={onDiscard}>
                    {words.discard}
                </Button>
            )}
            <Button
                type="submit"
                tone="ink"
                pending={waiting.some((entry) => entry.pending)}
                className="max-md:flex-1"
            >
                {words.save}
            </Button>
        </div>
    );
}
