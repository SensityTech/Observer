import { useQuery } from '@tanstack/react-query';
import { Link, Outlet, useParams } from '@tanstack/react-router';
import { ChevronDown, Clock, Search, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { featuresQuery } from '../api/features';
import { useMe } from '../auth/me';
import { Chip } from '../components/Chip';
import { buttonVariants } from '../components/Button';
import { inputClass } from '../components/Field';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf, CATEGORIES, GROUPS, opens, search, type CategoryId } from '../settings/catalog';

/**
 * Every setting of Observer in one place: its categories on the side, the one open beside them,
 * and a search through all of them that stands in for the category while something is typed.
 */
export function SettingsPage() {
    const { t } = useI18n();
    const me = useMe();
    const { category: current } = useParams({ strict: false });
    const [query, setQuery] = useState('');
    const [menuOpen, setMenuOpen] = useState(false);
    const field = useRef<HTMLInputElement>(null);
    const allowed = CATEGORIES.filter((category) => opens(me, category));
    const searching = query.trim() !== '';
    const shown = allowed.find(({ id }) => id === current);

    // `/` goes to the search from anywhere a key is not typed into, as on the mockup's hint.
    useEffect(() => {
        const toSearch = (event: KeyboardEvent) => {
            if (
                event.key !== '/' ||
                event.ctrlKey ||
                event.metaKey ||
                event.altKey ||
                event.defaultPrevented
            ) {
                return;
            }
            const target = event.target;
            if (
                target instanceof HTMLElement &&
                target.closest('input, textarea, select, [contenteditable]')
            ) {
                return;
            }
            event.preventDefault();
            field.current?.focus();
        };
        window.addEventListener('keydown', toSearch);
        return () => window.removeEventListener('keydown', toSearch);
    }, []);

    const leave = () => {
        setQuery('');
        setMenuOpen(false);
    };

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head items-center">
                <h1 className="page-title">{t.settings.title}</h1>
                <label className="relative block w-80 max-md:w-full">
                    <span className="sr-only">{t.settings.search.label}</span>
                    <Search
                        size={14}
                        strokeWidth={1.5}
                        className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink-3"
                        aria-hidden="true"
                    />
                    <input
                        ref={field}
                        type="search"
                        value={query}
                        placeholder={t.settings.search.placeholder}
                        aria-keyshortcuts="/"
                        className={cn(inputClass, 'pr-10 pl-8.5 [&::-webkit-search-cancel-button]:hidden')}
                        onChange={(event) => setQuery(event.target.value)}
                        onKeyDown={(event) => {
                            if (event.key === 'Escape' && query !== '') {
                                event.preventDefault();
                                setQuery('');
                            }
                        }}
                    />
                    {query === '' ? (
                        <span
                            className="kbd pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 max-md:hidden"
                            title={t.settings.search.shortcut}
                            aria-hidden="true"
                        >
                            /
                        </span>
                    ) : (
                        <button
                            type="button"
                            className={cn(
                                buttonVariants({ tone: 'ghost', size: 'icon-sm' }),
                                'absolute top-1/2 right-1 size-7.5 -translate-y-1/2 active:-translate-y-1/2 max-md:size-9',
                            )}
                            aria-label={t.settings.search.clear}
                            onClick={() => {
                                setQuery('');
                                field.current?.focus();
                            }}
                        >
                            <X size={14} strokeWidth={1.5} aria-hidden="true" />
                        </button>
                    )}
                </label>
            </div>

            <div className="flex flex-wrap items-start gap-6 max-md:gap-4">
                <nav
                    aria-label={t.settings.categoriesLabel}
                    className="flex min-w-0 flex-[0_0_13.75rem] flex-col max-md:flex-auto"
                >
                    <button
                        type="button"
                        className={cn(
                            buttonVariants({ tone: 'secondary' }),
                            'w-full justify-between md:hidden',
                        )}
                        aria-expanded={menuOpen}
                        aria-controls="settings-categories"
                        onClick={() => setMenuOpen((open) => !open)}
                    >
                        {t.settings.category(t.settings.categories[shown?.id ?? 'account'].name)}
                        <ChevronDown
                            size={16}
                            strokeWidth={1.5}
                            aria-hidden="true"
                            className={cn('text-ink-3 transition-transform', menuOpen && 'rotate-180')}
                        />
                    </button>
                    <div
                        id="settings-categories"
                        className={cn('flex flex-col gap-px max-md:mt-2', !menuOpen && 'max-md:hidden')}
                    >
                        {GROUPS.map((group) => {
                            const entries = allowed.filter((category) => category.group === group);
                            return (
                                entries.length > 0 && (
                                    <div key={group} className="group flex flex-col gap-px">
                                        <p className="px-3 pt-3.5 pb-1.5 font-mono text-micro tracking-group text-ink-3 uppercase group-first:pt-0">
                                            {t.settings.groups[group]}
                                        </p>
                                        {entries.map(({ id }) => (
                                            <CategoryLink key={id} id={id} onNavigate={leave} />
                                        ))}
                                    </div>
                                )
                            );
                        })}
                    </div>
                </nav>

                <div className="flex min-w-0 flex-[999_1_35rem] flex-col gap-4">
                    {searching && <Results query={query.trim()} onPick={leave} />}
                    {/* Hidden rather than gone: what is typed in a category waits for the search to end. */}
                    <div hidden={searching} className="flex flex-col gap-4">
                        <Outlet />
                    </div>
                </div>
            </div>
        </div>
    );
}

function CategoryLink({ id, onNavigate }: { id: CategoryId; onNavigate: () => void }) {
    const { t } = useI18n();
    return (
        <Link
            to="/settings/$category"
            params={{ category: id }}
            className="relative flex min-h-8.5 items-center rounded-[9px] px-3 text-nav font-medium text-ink-2 transition-colors hover:bg-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent aria-[current=page]:bg-active aria-[current=page]:text-ink aria-[current=page]:before:absolute aria-[current=page]:before:top-1/2 aria-[current=page]:before:-left-2 aria-[current=page]:before:-mt-[9px] aria-[current=page]:before:h-4.5 aria-[current=page]:before:w-[3px] aria-[current=page]:before:rounded-r-[3px] aria-[current=page]:before:bg-accent max-md:min-h-11 max-md:aria-[current=page]:before:hidden"
            onClick={onNavigate}
        >
            {t.settings.categories[id].name}
            {id === 'features' && <FeatureCount />}
        </Link>
    );
}

/** How many optional features the owner left on, beside their category. */
function FeatureCount() {
    const { t } = useI18n();
    const features = useQuery(featuresQuery).data?.features;
    if (features === undefined) {
        return null;
    }
    const on = features.filter((feature) => feature.enabled).length;
    return <Chip className="ml-auto">{t.settings.features.count(on, features.length)}</Chip>;
}

/** What the search found, by category, each leading to its row. */
function Results({ query, onPick }: { query: string; onPick: () => void }) {
    const { t } = useI18n();
    const me = useMe();
    const found = search(t, me, query);
    const categories = [...new Set(found.map((match) => match.category))];

    return (
        <section className="flex flex-col gap-4" aria-labelledby="settings-results">
            <h2 id="settings-results" className="text-figure-sm font-semibold text-ink">
                <output>
                    {found.length === 0
                        ? t.settings.search.none(query)
                        : t.settings.search.found(found.length, query)}
                </output>
            </h2>
            {categories.map((category) => (
                <section
                    key={category.id}
                    className="card overflow-hidden"
                    aria-labelledby={`results-${category.id}`}
                >
                    <h3 id={`results-${category.id}`} className="card-head text-card text-ink">
                        {t.settings.categories[category.id].name}
                    </h3>
                    <ul className="flex flex-col">
                        {found
                            .filter((match) => match.category === category)
                            .map(({ id, applies }) => (
                                <li key={id} className="border-t border-line first:border-t-0">
                                    <Link
                                        to="/settings/$category"
                                        params={{ category: category.id }}
                                        hash={anchorOf(id)}
                                        className="group flex flex-col gap-1 px-5 py-3.5 hover:bg-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent max-md:px-4"
                                        onClick={onPick}
                                    >
                                        <span className="text-body font-medium text-ink">
                                            {t.settings.items[id].label}
                                        </span>
                                        <span className="text-small text-pretty text-ink-3 group-hover:text-ink-2">
                                            {t.settings.items[id].about}
                                        </span>
                                        {applies !== undefined && (
                                            <span className="flex items-center gap-1.5 text-meta text-ink-3 group-hover:text-ink-2">
                                                <Clock size={12} strokeWidth={2} aria-hidden="true" />
                                                {t.settings.applies[applies]}
                                            </span>
                                        )}
                                    </Link>
                                </li>
                            ))}
                    </ul>
                </section>
            ))}
        </section>
    );
}
