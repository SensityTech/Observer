// Markdown as the panel reads it, for release notes and the documentation: headings, lists,
// paragraphs, fenced code, tables, emphasis and links. What it yields is drawn as React elements,
// never as HTML.

/** Each part carries its place in the text as its key: a text never moves once read. */
export type Block = { key: string } & (
    | { kind: 'heading'; level: number; text: string }
    | { kind: 'list'; ordered: boolean; items: { key: string; text: string }[] }
    | { kind: 'code'; text: string; lang: string }
    | { kind: 'table'; head: Cell[]; rows: { key: string; cells: Cell[] }[] }
    | { kind: 'quote'; text: string }
    | { kind: 'rule' }
    | { kind: 'paragraph'; text: string }
);

/** A cell of a table, in Markdown. */
export type Cell = { key: string; text: string };

export type Inline = { key: string } & (
    | { kind: 'text'; text: string }
    | { kind: 'code'; text: string }
    | { kind: 'strong'; children: Inline[] }
    | { kind: 'em'; children: Inline[] }
    | { kind: 'link'; href: string; children: Inline[] }
    /** A link inside the text's own place: `#section`, or a path beside it. Its reader decides where it leads. */
    | { kind: 'local'; target: string; children: Inline[] }
);

type Unkeyed<T> = T extends unknown ? Omit<T, 'key'> : never;

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}\d{1,9}[.)]\s+(.*)$/;
const FENCE = /^\s{0,3}(```|~~~)\s*([\w+-]*)/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE_ROW = /^\s{0,3}\|.*\|\s*$/;
const TABLE_RULE = /^\s{0,3}\|(\s*:?-+:?\s*\|)+\s*$/;

/** The cells of a table's line. A `|` inside a code span belongs to its cell. */
const cellsOf = (line: string): string[] => {
    const cells: string[] = [];
    let cell = '';
    let code = false;
    for (const char of line.trim().replace(/^\|/, '').replace(/\|$/, '')) {
        if (char === '`') {
            code = !code;
        }
        if (char === '|' && !code) {
            cells.push(cell.trim());
            cell = '';
        } else {
            cell += char;
        }
    }
    cells.push(cell.trim());
    return cells;
};

export function parseBlocks(source: string): Block[] {
    const lines = source.replace(/\r\n?/g, '\n').split('\n');
    const blocks: Block[] = [];
    const add = (block: Unkeyed<Block>) => blocks.push({ ...block, key: `b${blocks.length}` } as Block);
    let paragraph: string[] = [];
    const flush = () => {
        if (paragraph.length > 0) {
            add({ kind: 'paragraph', text: paragraph.join(' ') });
            paragraph = [];
        }
    };
    for (let at = 0; at < lines.length; at += 1) {
        const line = lines[at] ?? '';
        if (line.trim() === '') {
            flush();
            continue;
        }
        const fence = FENCE.exec(line);
        if (fence !== null) {
            flush();
            const marker = fence[1] ?? '```';
            const code: string[] = [];
            at += 1;
            while (at < lines.length && !(lines[at] ?? '').trimStart().startsWith(marker)) {
                code.push(lines[at] ?? '');
                at += 1;
            }
            add({ kind: 'code', text: code.join('\n'), lang: fence[2] ?? '' });
            continue;
        }
        const heading = HEADING.exec(line);
        if (heading !== null) {
            flush();
            add({ kind: 'heading', level: (heading[1] ?? '#').length, text: heading[2] ?? '' });
            continue;
        }
        if (TABLE_ROW.test(line) && TABLE_RULE.test(lines[at + 1] ?? '')) {
            flush();
            const key = `b${blocks.length}`;
            const head = cellsOf(line).map((text, index) => ({ key: `${key}.h${index}`, text }));
            const rows: { key: string; cells: Cell[] }[] = [];
            at += 2;
            while (at < lines.length && TABLE_ROW.test(lines[at] ?? '')) {
                const cells = cellsOf(lines[at] ?? '');
                const row = `${key}.${rows.length}`;
                // As many cells as the head has, whatever the line holds.
                rows.push({
                    key: row,
                    cells: head.map((_, index) => ({ key: `${row}.${index}`, text: cells[index] ?? '' })),
                });
                at += 1;
            }
            at -= 1;
            add({ kind: 'table', head, rows });
            continue;
        }
        if (RULE.test(line)) {
            flush();
            add({ kind: 'rule' });
            continue;
        }
        const item = BULLET.exec(line) ?? NUMBERED.exec(line);
        if (item !== null) {
            flush();
            const ordered = NUMBERED.test(line) && !BULLET.test(line);
            const text = item[1] ?? '';
            const last = blocks.at(-1);
            if (last?.kind === 'list' && last.ordered === ordered) {
                last.items.push({ key: `${last.key}.${last.items.length}`, text });
            } else {
                add({ kind: 'list', ordered, items: [{ key: `b${blocks.length}.0`, text }] });
            }
            continue;
        }
        const quote = QUOTE.exec(line);
        if (quote !== null) {
            flush();
            const text = quote[1] ?? '';
            const last = blocks.at(-1);
            if (last?.kind === 'quote') {
                last.text = `${last.text} ${text}`;
            } else {
                add({ kind: 'quote', text });
            }
            continue;
        }
        // A line that carries on the list item above it.
        const last = blocks.at(-1);
        if (paragraph.length === 0 && last?.kind === 'list' && /^\s{2,}\S/.test(line)) {
            const carried = last.items.at(-1);
            if (carried !== undefined) {
                carried.text = `${carried.text} ${line.trim()}`;
            }
            continue;
        }
        paragraph.push(line.trim());
    }
    flush();
    return blocks;
}

const CODE = /^`([^`]+)`/;
const LINK = /^\[([^\]]+)\]\(([^)\s]+)\)/;
const ANGLED = /^<(https?:\/\/[^>\s]+)>/;
const BARE = /^https?:\/\/[^\s<>]+/;
const STRONG = /^(\*\*|__)(?=\S)(.+?)(?<=\S)\1/;
const EM = /^\*(?=[^\s*])(.+?)(?<=[^\s*])\*/;
// A place in the same text, or a path beside it: no scheme, so never an address of its own.
const LOCAL = /^(?:#[\w-]+|[\w-][\w./-]*(?:#[\w-]+)?)$/;

export function parseInline(text: string): Inline[] {
    const nodes: Inline[] = [];
    const keyed = (node: Unkeyed<Inline>) => nodes.push({ ...node, key: `i${nodes.length}` } as Inline);
    let plain = '';
    const push = (node: Unkeyed<Inline>) => {
        if (plain !== '') {
            keyed({ kind: 'text', text: plain });
            plain = '';
        }
        keyed(node);
    };
    let at = 0;
    while (at < text.length) {
        const rest = text.slice(at);
        const char = text.charAt(at);
        let match: RegExpExecArray | null = null;
        if (char === '`' && (match = CODE.exec(rest)) !== null) {
            push({ kind: 'code', text: match[1] ?? '' });
        } else if (char === '[' && (match = LINK.exec(rest)) !== null) {
            const target = match[2] ?? '';
            const href = safeHref(target);
            if (href !== null) {
                push({ kind: 'link', href, children: parseInline(match[1] ?? '') });
            } else if (LOCAL.test(target)) {
                push({ kind: 'local', target, children: parseInline(match[1] ?? '') });
            } else {
                plain += match[1] ?? '';
            }
        } else if (char === '<' && (match = ANGLED.exec(rest)) !== null) {
            push(urlLink(match[1] ?? ''));
        } else if (
            char === 'h' &&
            /\w/.test(text[at - 1] ?? ' ') === false &&
            (match = BARE.exec(rest)) !== null
        ) {
            // A sentence's last stop or a closing bracket is not part of the address.
            const url = match[0].replace(/[.,;:!?'")\]]+$/, '');
            push(urlLink(url));
            at += url.length;
            continue;
        } else if ((char === '*' || char === '_') && (match = STRONG.exec(rest)) !== null) {
            push({ kind: 'strong', children: parseInline(match[2] ?? '') });
        } else if (char === '*' && (match = EM.exec(rest)) !== null) {
            push({ kind: 'em', children: parseInline(match[1] ?? '') });
        }
        if (match === null) {
            plain += char;
            at += 1;
        } else {
            at += match[0].length;
        }
    }
    if (plain !== '') {
        keyed({ kind: 'text', text: plain });
    }
    return nodes;
}

/** What the nodes say, without their markup. */
export const plainText = (nodes: Inline[]): string =>
    nodes.map((node) => ('text' in node ? node.text : plainText(node.children))).join('');

/** An http or https address, or null for anything else (`javascript:`, a relative path…). */
const safeHref = (href: string): string | null => {
    try {
        const url = new URL(href);
        return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : null;
    } catch {
        return null;
    }
};

const GITHUB_ITEM = /^https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/(?:pull|issues)\/(\d+)$/;
const GITHUB_COMPARE = /^https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/compare\/(\S+)$/;

/** A pull request, an issue or a comparison on GitHub reads as GitHub shows it, `#45`. */
const urlLink = (url: string): Unkeyed<Inline> => {
    const href = safeHref(url);
    if (href === null) {
        return { kind: 'text', text: url };
    }
    const item = GITHUB_ITEM.exec(url);
    const compare = GITHUB_COMPARE.exec(url);
    const text = item !== null ? `#${item[1] ?? ''}` : compare !== null ? (compare[1] ?? url) : url;
    return { kind: 'link', href, children: [{ key: 'i0', kind: 'text', text }] };
};
