// DESIGN.md's table recipe. A wide table scrolls inside its card, never the page.
export const table = {
    scroll: 'relative overflow-x-auto rounded-b-card',
    table: 'w-full border-separate border-spacing-0 text-ui',
    th: 'h-9 border-b border-line px-3 text-left text-meta font-medium whitespace-nowrap text-ink-3 first:pl-5 last:pr-5 max-md:first:pl-4 max-md:last:pr-4',
    row: 'hover:bg-surface-2 [&:last-child>td]:border-b-0',
    td: 'h-10 border-b border-line px-3 py-1.5 text-ink-2 first:pl-5 last:pr-5 max-md:first:pl-4 max-md:last:pr-4',
    mono: 'font-mono text-small whitespace-nowrap text-ink [font-variant-ligatures:none]',
    sub: 'mt-0.75 flex flex-wrap items-center gap-1.5 text-meta text-ink-3',
};
