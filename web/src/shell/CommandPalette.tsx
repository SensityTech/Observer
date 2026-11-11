import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { BookOpen, Contact, Search, SlidersHorizontal, Wrench, type LucideIcon } from 'lucide-react';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { integrationDocsQuery } from '../api/docs';
import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { onlineQuery } from '../api/players';
import { allows } from '../auth/me';
import { buildDocs, docIndex } from '../docs/model';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf, search as searchSettings } from '../settings/catalog';
import { navSections, opensEntry } from './nav';

/** Results of each kind shown under a query. */
const MAX_OF_KIND = 8;

type Result = {
    key: string;
    icon: LucideIcon;
    label: string;
    /** What kind of place it is, or where it lies. */
    hint: string;
    /** A name of the code, an event's or an export's, shown as code is. */
    code?: boolean;
    go: () => void;
};

const folded = (text: string) =>
    text
        .normalize('NFD')
        .replace(/\p{Diacritic}/gu, '')
        .toLowerCase();

/** Whether the keys pressed ask for the palette: Ctrl K, or ⌘ K on a Mac. */
const asksForPalette = (event: KeyboardEvent) =>
    (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'k';

/**
 * Goes anywhere in the panel from the keyboard: a page, a setting by its name, a section, an event
 * or an export of the documentation, a player who is connected. Ctrl K opens it from any page; it
 * lists only what the account may open.
 */
export function CommandPalette({
    me,
    open,
    onOpen,
    onClose,
}: {
    me: Me;
    open: boolean;
    onOpen: () => void;
    onClose: () => void;
}) {
    useEffect(() => {
        const toggle = (event: KeyboardEvent) => {
            if (asksForPalette(event)) {
                event.preventDefault();
                if (open) {
                    onClose();
                } else {
                    onOpen();
                }
            }
        };
        window.addEventListener('keydown', toggle);
        return () => window.removeEventListener('keydown', toggle);
    }, [open, onOpen, onClose]);

    return open ? <Palette me={me} onClose={onClose} /> : null;
}

function Palette({ me, onClose }: { me: Me; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.palette;
    const navigate = useNavigate();
    const dialog = useRef<HTMLDialogElement>(null);
    const listId = useId();
    const [query, setQuery] = useState('');
    const [active, setActive] = useState(0);
    const seesPlayers = allows(me, Permission.PlayersView);
    const online = useQuery({ ...onlineQuery, enabled: seesPlayers, refetchInterval: false });
    // The contract only changes with the build: its sections are read once.
    const contract = useQuery({ ...integrationDocsQuery, staleTime: Infinity }).data;
    const documented = useMemo(
        () => (contract === undefined ? [] : docIndex(buildDocs(contract))),
        [contract],
    );

    useLayoutEffect(() => {
        const element = dialog.current;
        element?.showModal();
        return () => element?.close();
    }, []);

    useEffect(() => {
        const element = dialog.current;
        if (element === null) {
            return;
        }
        // The box fills its element: a click on the element itself hit the backdrop.
        const outside = (event: MouseEvent) => {
            if (event.target === element) {
                onClose();
            }
        };
        element.addEventListener('click', outside);
        return () => element.removeEventListener('click', outside);
    }, [onClose]);

    const leave = (go: () => void) => () => {
        onClose();
        go();
    };
    const sections = navSections(t);
    const pages: Result[] = [
        ...sections.main,
        ...sections.people,
        ...sections.server,
        ...sections.insights,
        ...sections.debug,
        ...sections.maintenance,
        ...sections.settings,
    ]
        .filter((entry) => opensEntry(me, entry))
        .map((entry) => ({
            key: `page ${entry.to}`,
            icon: entry.icon,
            label: entry.label,
            hint: words.page,
            go: leave(() => void navigate({ to: entry.to })),
        }));
    if (me.isOwner) {
        const tool = (
            to: '/tools/duplicate-archetypes' | '/tools/heavy-assets' | '/tools/missing-files',
            label: string,
        ) =>
            pages.push({
                key: `page ${to}`,
                icon: Wrench,
                label,
                hint: words.tool,
                go: leave(() => void navigate({ to })),
            });
        tool('/tools/duplicate-archetypes', t.tools.archetypes.title);
        tool('/tools/heavy-assets', t.tools.hub.heavy.title);
        tool('/tools/missing-files', t.tools.hub.missing.title);
    }

    const wanted = folded(query.trim());
    const asked = wanted.split(/\s+/u).filter(Boolean);
    /** Whether each word asked for starts a word of the text: `hang` is not in `changes`. */
    const matches = (text: string) => {
        const parts = folded(text).split(/[^\p{L}\p{N}]+/u);
        return asked.every((word) => parts.some((part) => part.startsWith(word)));
    };
    const results: Result[] =
        wanted === ''
            ? pages
            : [
                  ...pages.filter((page) => matches(page.label)),
                  // `restartScheduled` is also found by `scheduled`: a name of the code is read word by word.
                  ...documented
                      .filter(({ label }) => matches(`${label} ${label.replace(/([a-z])([A-Z])/g, '$1 $2')}`))
                      .slice(0, MAX_OF_KIND)
                      .map((match) => ({
                          key: `doc ${match.kind} ${match.label}`,
                          icon: BookOpen,
                          label: match.label,
                          hint: words.doc(words.docKinds[match.kind]),
                          code: match.kind !== 'section',
                          go: leave(() => void navigate({ to: '/docs', hash: match.id })),
                      })),
                  // Those named by what was asked first, then those whose explanation says it.
                  ...searchSettings(t, me, query)
                      .map((match) => {
                          const { label, about } = t.settings.items[match.id];
                          return { match, rank: matches(label) ? 0 : matches(about) ? 1 : 2 };
                      })
                      .toSorted((left, right) => left.rank - right.rank)
                      .map(({ match }) => match)
                      .slice(0, MAX_OF_KIND)
                      .map(({ category, id }) => ({
                          key: `setting ${id}`,
                          icon: SlidersHorizontal,
                          label: t.settings.items[id].label,
                          hint: words.setting(t.settings.categories[category.id].name),
                          go: leave(
                              () =>
                                  void navigate({
                                      to: '/settings/$category',
                                      params: { category: category.id },
                                      hash: anchorOf(id),
                                  }),
                          ),
                      })),
                  // Read for this account alone: another one's list may still be in memory.
                  ...((seesPlayers ? online.data?.players : undefined) ?? [])
                      .filter(
                          (player) =>
                              player.player !== null &&
                              matches(`${player.name ?? ''} ${player.label?.name ?? ''} ${player.netId}`),
                      )
                      .slice(0, MAX_OF_KIND)
                      .map((player) => ({
                          key: `player ${player.netId}`,
                          icon: Contact,
                          label: player.name ?? t.map.unnamed(player.netId),
                          hint: words.player(player.netId),
                          go: leave(
                              () =>
                                  void navigate({
                                      to: '/players/$id',
                                      params: { id: String(player.player) },
                                  }),
                          ),
                      })),
              ];
    const current = Math.min(active, Math.max(results.length - 1, 0));

    return (
        <dialog
            ref={dialog}
            aria-label={words.label}
            className="mx-auto mt-[12dvh] mb-auto max-h-[70dvh] w-[calc(100%-2rem)] max-w-[36rem] flex-col overflow-clip rounded-dialog border border-line bg-surface p-0 text-ink shadow-lg backdrop:bg-overlay open:flex"
            onCancel={(event) => {
                event.preventDefault();
                onClose();
            }}
        >
            <div className="flex items-center gap-3 border-b border-line px-4">
                <Search size={16} strokeWidth={1.5} className="flex-none text-ink-3" aria-hidden="true" />
                <input
                    type="search"
                    aria-label={words.label}
                    aria-controls={listId}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder={words.placeholder}
                    value={query}
                    className="h-12 min-w-0 flex-1 bg-transparent text-body text-ink outline-none placeholder:text-ink-3 [&::-webkit-search-cancel-button]:hidden"
                    onChange={(event) => {
                        setQuery(event.target.value);
                        setActive(0);
                    }}
                    onKeyDown={(event) => {
                        if (event.key === 'ArrowDown') {
                            event.preventDefault();
                            setActive(results.length === 0 ? 0 : (current + 1) % results.length);
                        } else if (event.key === 'ArrowUp') {
                            event.preventDefault();
                            setActive(
                                results.length === 0 ? 0 : (current - 1 + results.length) % results.length,
                            );
                        } else if (event.key === 'Enter') {
                            event.preventDefault();
                            results[current]?.go();
                        }
                    }}
                />
                <kbd className="flex-none rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-micro text-ink-3">
                    Esc
                </kbd>
            </div>
            <ul
                id={listId}
                aria-label={words.results}
                className="flex min-h-0 flex-col overflow-y-auto overscroll-contain p-1.5 scrollbar-thin"
            >
                {results.map((result, index) => {
                    const Icon = result.icon;
                    return (
                        <li key={result.key}>
                            <button
                                type="button"
                                aria-current={index === current ? 'true' : undefined}
                                className="flex min-h-10 w-full items-center gap-3 rounded-control px-3 py-2 text-left aria-[current=true]:bg-surface-3"
                                onPointerMove={() => setActive(index)}
                                onFocus={() => setActive(index)}
                                onClick={result.go}
                            >
                                <Icon
                                    size={16}
                                    strokeWidth={1.5}
                                    className="flex-none text-ink-3"
                                    aria-hidden="true"
                                />
                                <span
                                    className={cn(
                                        'min-w-0 flex-1 truncate text-ui text-ink',
                                        result.code === true &&
                                            'font-mono text-small [font-variant-ligatures:none]',
                                    )}
                                >
                                    {result.label}
                                </span>
                                <span className="flex-none text-meta text-ink-3">{result.hint}</span>
                            </button>
                        </li>
                    );
                })}
                {results.length === 0 && (
                    <li className="px-3 py-6 text-center text-small text-ink-3">{words.none}</li>
                )}
            </ul>
        </dialog>
    );
}
