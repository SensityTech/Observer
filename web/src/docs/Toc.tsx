import { ChevronDown } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';

import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Anchor } from './Anchor';

export type TocItem = {
    key: string;
    /** Where it leads on the page. */
    id: string;
    label: string;
    /** An event or an export, under its section. */
    sub: boolean;
};

export type TocGroup = { label: string; items: TocItem[] };

/** How far under where a link brings a section its top may be for the section to count as reached. */
const SLACK = 24;

/**
 * The place of the page that is under the reader's eyes: the last of `ids` whose top went past
 * where a link to it would bring it, the last one once the page is at its end.
 */
export const useReached = (ids: string[]): string | null => {
    const [reached, setReached] = useState<string | null>(null);
    useEffect(() => {
        const read = () => {
            let found = ids[0] ?? null;
            for (const id of ids) {
                const element = document.getElementById(id);
                if (element === null) {
                    continue;
                }
                const line = (Number.parseFloat(getComputedStyle(element).scrollMarginTop) || 0) + SLACK;
                if (element.getBoundingClientRect().top <= line) {
                    found = id;
                }
            }
            const page = document.documentElement;
            if (
                ids.length > 0 &&
                page.scrollTop > 0 &&
                innerHeight + page.scrollTop >= page.scrollHeight - 2
            ) {
                found = ids.at(-1) ?? found;
            }
            setReached(found);
        };
        read();
        addEventListener('scroll', read, { passive: true });
        addEventListener('resize', read);
        return () => {
            removeEventListener('scroll', read);
            removeEventListener('resize', read);
        };
    }, [ids]);
    return reached;
};

const entry =
    'flex min-h-8 items-center rounded-[9px] px-3 text-nav font-medium text-ink-2 transition-colors hover:bg-hover hover:text-ink focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent aria-[current=location]:bg-active aria-[current=location]:text-ink max-md:min-h-11';
const subEntry =
    'min-h-7 pl-6 font-mono text-meta font-normal text-ink-3 [font-variant-ligatures:none] aria-[current=location]:text-ink';

function Entries({
    groups,
    reached,
    onNavigate,
}: {
    groups: TocGroup[];
    reached: string | null;
    onNavigate?: () => void;
}) {
    // Exports share their section's place: the section alone is marked.
    const current = groups.flatMap((group) => group.items).find((item) => item.id === reached)?.key;
    return groups.map((group) => (
        <div key={group.label} className="group flex flex-col gap-px">
            <p className="px-3 pt-3.5 pb-1.5 font-mono text-micro tracking-group text-ink-3 uppercase group-first:pt-0">
                {group.label}
            </p>
            {group.items.map((item) => (
                <Anchor
                    key={item.key}
                    id={item.id}
                    aria-current={item.key === current ? 'location' : undefined}
                    className={cn(entry, item.sub && subEntry)}
                    onClick={onNavigate}
                >
                    <span className="min-w-0 truncate">{item.label}</span>
                </Anchor>
            ))}
        </div>
    ));
}

/**
 * The documentation's contents: beside the page on a wide screen, where it follows the reader, and
 * under one button on a phone, which names the section being read.
 */
export function Toc({ groups }: { groups: TocGroup[] }) {
    const { t } = useI18n();
    const listId = useId();
    const ids = useMemo(
        () => [...new Set(groups.flatMap((group) => group.items.map((item) => item.id)))],
        [groups],
    );
    const reached = useReached(ids);
    const [open, setOpen] = useState(false);
    const side = useRef<HTMLElement>(null);
    const phone = useRef<HTMLDivElement>(null);
    const items = groups.flatMap((group) => group.items);
    const reading =
        items.find((item) => item.id === reached && !item.sub) ?? items.find((item) => item.id === reached);

    // A long contents list scrolls by itself: the entry being read stays in sight.
    useEffect(() => {
        const list = side.current;
        const marked = reached === null ? null : list?.querySelector('[aria-current="location"]');
        if (list === null || marked === null || marked === undefined) {
            return;
        }
        const [box, row] = [list.getBoundingClientRect(), marked.getBoundingClientRect()];
        if (row.top < box.top) {
            list.scrollTop -= box.top - row.top + 8;
        } else if (row.bottom > box.bottom) {
            list.scrollTop += row.bottom - box.bottom + 8;
        }
    }, [reached]);

    useEffect(() => {
        if (!open) {
            return;
        }
        const close = (event: Event) => {
            if (
                event instanceof KeyboardEvent
                    ? event.key === 'Escape'
                    : !phone.current?.contains(event.target as Node)
            ) {
                setOpen(false);
            }
        };
        addEventListener('keydown', close);
        addEventListener('pointerdown', close);
        return () => {
            removeEventListener('keydown', close);
            removeEventListener('pointerdown', close);
        };
    }, [open]);

    return (
        <>
            <nav
                ref={side}
                aria-label={t.docs.toc}
                className="sticky top-3 flex max-h-[calc(100dvh-1.5rem)] flex-[0_0_12.5rem] flex-col overflow-y-auto scrollbar-none max-xl:top-[calc(var(--mobilebar-height)+0.75rem)] max-xl:max-h-[calc(100dvh-var(--mobilebar-height)-1.5rem)] max-md:hidden"
            >
                <Entries groups={groups} reached={reached} />
            </nav>
            <div
                ref={phone}
                // A band as wide as the screen: what scrolls under the button does not show around it.
                className="sticky top-(--mobilebar-height) z-(--z-dock) -mx-4 bg-bg px-4 py-2 grain md:hidden"
            >
                <button
                    type="button"
                    className={cn(buttonVariants({ tone: 'secondary' }), 'w-full justify-between')}
                    aria-expanded={open}
                    aria-controls={listId}
                    onClick={() => setOpen(!open)}
                >
                    <span className="min-w-0 truncate">
                        <span className="font-normal text-ink-3">{t.docs.toc}</span>
                        {reading !== undefined && <> · {reading.label}</>}
                    </span>
                    <ChevronDown
                        size={16}
                        strokeWidth={1.5}
                        aria-hidden="true"
                        className={cn('flex-none text-ink-3 transition-transform', open && 'rotate-180')}
                    />
                </button>
                <nav
                    id={listId}
                    aria-label={t.docs.toc}
                    hidden={!open}
                    className="absolute inset-x-4 top-full -mt-0.5 flex max-h-[60dvh] flex-col overflow-y-auto overscroll-contain rounded-box border border-line-strong bg-surface p-1.5 pt-3 shadow-pop scrollbar-thin"
                >
                    <Entries groups={groups} reached={reached} onNavigate={() => setOpen(false)} />
                </nav>
            </div>
        </>
    );
}
