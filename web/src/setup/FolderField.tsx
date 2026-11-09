import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { CircleCheck, Folder, Lock } from 'lucide-react';
import { useEffect, useId, useState, type KeyboardEvent, type ReactNode } from 'react';

import { folderQuery } from '../api/serverSetup';
import { Chip } from '../components/Chip';
import { TextField } from '../components/Field';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { choosable, intoFolder, suggestionsOf, typedFolder, type Suggestion } from './paths';

/** How long typing pauses before the folder typed is listed. */
const PAUSE_MS = 150;

/**
 * The next option that can be chosen from `from`, by `step`, round the list: from none, down goes
 * to the first and up to the last. -1 when none can be.
 */
const next = (options: Suggestion[], from: number, step: 1 | -1) => {
    const count = options.length;
    const start = from === -1 && step === -1 ? 0 : from;
    for (let moved = 1; moved <= count; moved++) {
        const index = (((start + step * moved) % count) + count) % count;
        const option = options[index];
        if (option !== undefined && choosable(option.entry)) {
            return index;
        }
    }
    return -1;
};

/**
 * The server folder's path, typed with the host's folders listed below it as it goes: a combobox.
 * The arrows move through them, Tab completes the one shown, Enter chooses it, or the path typed
 * when none is, and has it checked.
 */
export function FolderField({
    value,
    error,
    beside,
    onChange,
    onChoose,
}: {
    value: string;
    error: string | undefined;
    beside?: ReactNode;
    onChange: (value: string) => void;
    onChoose: (path: string) => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const id = useId();
    const listId = `${id}-folders`;
    const [open, setOpen] = useState(false);
    const [active, setActive] = useState(-1);
    const [asked, setAsked] = useState(value);
    useEffect(() => {
        const timer = setTimeout(() => setAsked(value), PAUSE_MS);
        return () => clearTimeout(timer);
    }, [value]);

    const typed = typedFolder(asked);
    const listing = useQuery({
        ...folderQuery({ path: typed?.parent ?? '', prefix: typed?.prefix ?? '' }),
        enabled: open && typed !== null,
        placeholderData: keepPreviousData,
    });
    const report = listing.data;
    // The listing asked a moment ago, narrowed to what was typed since: the next one replaces it.
    const options =
        report === undefined || report.problem !== null || typedFolder(value) === null
            ? []
            : suggestionsOf(report).filter((option) =>
                  option.entry.path.toLowerCase().startsWith(value.toLowerCase()),
              );
    const answered = report !== undefined && report.problem === null && !listing.isPlaceholderData;
    const expanded = open && typedFolder(value) !== null && (options.length > 0 || answered);
    const current = active < options.length ? active : -1;

    const choose = (path: string) => {
        onChange(path);
        setOpen(false);
        setActive(-1);
        onChoose(path);
    };
    // A server is the end of the path; any other folder is gone into, its own folders listed next.
    const complete = (option: Suggestion) => {
        const server = option.entry.kind === 'server';
        const path = server ? option.entry.path : intoFolder(option.entry.path);
        onChange(path);
        setAsked(path);
        setActive(-1);
        setOpen(!server);
    };

    const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            if (!expanded) {
                setOpen(true);
                return;
            }
            setActive(next(options, current, event.key === 'ArrowDown' ? 1 : -1));
        } else if (event.key === 'Tab' && !event.shiftKey && expanded) {
            const option = options[current] ?? options.find((each) => choosable(each.entry));
            if (option !== undefined) {
                event.preventDefault();
                complete(option);
            }
        } else if (event.key === 'Enter') {
            event.preventDefault();
            const option = expanded ? options[current] : undefined;
            choose(option?.entry.path ?? value.trim());
        } else if (event.key === 'Escape' && expanded) {
            event.preventDefault();
            setOpen(false);
        }
    };

    return (
        <TextField
            label={words.label}
            hint={words.hint}
            value={value}
            error={error}
            placeholder={words.placeholder}
            spellCheck={false}
            autoCapitalize="none"
            autoComplete="off"
            // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- an ARIA combobox: a select cannot list what each folder is, nor take a typed path
            role="combobox"
            aria-expanded={expanded}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={current === -1 ? undefined : `${id}-option-${current}`}
            inputClassName="h-10.5 font-mono text-ui md:pr-30"
            onChange={(event) => {
                onChange(event.target.value);
                setOpen(true);
                setActive(-1);
            }}
            onKeyDown={onKeyDown}
            onBlur={() => setOpen(false)}
            beside={beside}
            after={
                <>
                    <span
                        className="pointer-events-none absolute top-1/2 right-3 hidden -translate-y-1/2 items-center gap-1.5 text-small text-ink-3 md:inline-flex"
                        aria-hidden="true"
                    >
                        <kbd className="kbd">{words.keys.tab}</kbd>
                        {words.complete}
                    </span>
                    <div
                        className={cn(
                            'absolute top-full right-0 left-0 z-30 mt-1.5 rounded-box border border-line-strong bg-surface p-1.5 shadow-pop',
                            !expanded && 'hidden',
                        )}
                    >
                        <div
                            // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- the combobox's own listbox, which the field drives
                            role="listbox"
                            id={listId}
                            aria-label={words.suggestions}
                            className="max-h-76 overflow-y-auto scrollbar-thin"
                        >
                            {options.map((option, index) => (
                                <Option
                                    key={option.entry.path}
                                    id={`${id}-option-${index}`}
                                    option={option}
                                    active={index === current}
                                    onPoint={() => setActive(index)}
                                    onChoose={() => choose(option.entry.path)}
                                />
                            ))}
                        </div>
                        {options.length === 0 && (
                            <p className="px-2.5 py-2 text-small text-ink-3">{words.noMatch}</p>
                        )}
                        <div
                            className="mt-1 hidden flex-wrap items-center gap-x-3.5 gap-y-1 border-t border-line px-2.5 pt-2 pb-1 text-small text-ink-3 md:flex"
                            aria-hidden="true"
                        >
                            <span className="inline-flex items-center gap-1.25">
                                <kbd className="kbd">{words.keys.up}</kbd>
                                <kbd className="kbd">{words.keys.down}</kbd>
                                {words.move}
                            </span>
                            <span className="inline-flex items-center gap-1.25">
                                <kbd className="kbd">{words.keys.tab}</kbd>
                                {words.complete}
                            </span>
                            <span className="inline-flex items-center gap-1.25">
                                <kbd className="kbd">{words.keys.enter}</kbd>
                                {words.chooseAndCheck}
                            </span>
                        </div>
                    </div>
                </>
            }
            below={
                <output className="sr-only">
                    {expanded && report !== undefined ? words.shown(options.length, report.truncated) : ''}
                </output>
            }
        />
    );
}

/** One folder offered: what it is in a word, and what it holds. */
function Option({
    id,
    option,
    active,
    onPoint,
    onChoose,
}: {
    id: string;
    option: Suggestion;
    active: boolean;
    onPoint: () => void;
    onChoose: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const { entry } = option;
    const enabled = choosable(entry);
    const Icon = entry.kind === 'server' ? CircleCheck : entry.kind === 'unreadable' ? Lock : Folder;
    const serverInside = entry.inside.some((inside) => inside.kind === 'server');
    const detail =
        entry.kind === 'server'
            ? words.serverDetail(entry.resources, entry.cfg)
            : entry.kind === 'txAdminData'
              ? words.txAdminDetail
              : entry.kind === 'folder' && entry.folders !== null
                ? words.folderDetail(entry.folders, serverInside)
                : null;

    // The keyboard stays in the field, which moves through the options: a press only picks one.
    return (
        // oxlint-disable-next-line jsx-a11y/click-events-have-key-events -- the field's keys choose it
        <div
            id={id}
            // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- an option of the combobox's listbox
            role="option"
            tabIndex={-1}
            aria-selected={active}
            aria-disabled={!enabled || undefined}
            className={cn(
                'flex flex-wrap items-center gap-x-2.5 gap-y-0.5 rounded-sm px-2.5 py-2 max-md:min-h-11',
                enabled ? 'cursor-pointer' : 'cursor-not-allowed text-ink-3',
                active && 'bg-accent-soft',
            )}
            // The field keeps the keyboard: a press on an option must not blur it first.
            onMouseDown={(event) => event.preventDefault()}
            onMouseMove={enabled ? onPoint : undefined}
            onClick={enabled ? onChoose : undefined}
        >
            <Icon
                size={14}
                strokeWidth={1.5}
                className={cn('flex-none', entry.kind === 'server' ? 'text-success' : 'text-ink-3')}
                aria-hidden="true"
            />
            <span className="min-w-0 font-mono text-ui [overflow-wrap:anywhere]">
                <b className="font-semibold">{option.head}</b>
                {option.tail}
            </span>
            {entry.link && <span className="text-small text-ink-3">{words.link}</span>}
            <span className="ml-auto flex flex-wrap items-center justify-end gap-x-2 gap-y-0.5">
                {entry.kind === 'server' && <Chip tone="success">{words.kinds.server}</Chip>}
                {entry.kind === 'txAdminData' && <Chip>{words.kinds.txAdminData}</Chip>}
                {entry.kind === 'unreadable' && <span className="text-small">{words.kinds.unreadable}</span>}
                {detail !== null && <span className="text-small text-ink-3">{detail}</span>}
            </span>
        </div>
    );
}
