import { Fragment, type ReactNode } from 'react';

/**
 * Release notes as GitHub writes them, drawn as React elements and never as HTML: headings,
 * lists, paragraphs, code, emphasis and links. A link leads only to an http or https address; a
 * pull request, an issue or a comparison on GitHub reads as GitHub shows it, `#45`.
 */
export function Markdown({ source }: { source: string }) {
    return (
        <div className="flex flex-col gap-2.5 text-body text-ink-2">
            {parseBlocks(source).map((block) => (
                <BlockView key={block.key} block={block} />
            ))}
        </div>
    );
}

/** Each part carries its place in the notes as its key: the notes never move once read. */
export type Block = { key: string } & (
    | { kind: 'heading'; level: number; text: string }
    | { kind: 'list'; ordered: boolean; items: { key: string; text: string }[] }
    | { kind: 'code'; text: string }
    | { kind: 'quote'; text: string }
    | { kind: 'rule' }
    | { kind: 'paragraph'; text: string }
);

export type Inline = { key: string } & (
    | { kind: 'text'; text: string }
    | { kind: 'code'; text: string }
    | { kind: 'strong'; children: Inline[] }
    | { kind: 'em'; children: Inline[] }
    | { kind: 'link'; href: string; children: Inline[] }
);

type Unkeyed<T> = T extends unknown ? Omit<T, 'key'> : never;

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}\d{1,9}[.)]\s+(.*)$/;
const FENCE = /^\s{0,3}(```|~~~)/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;

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
            add({ kind: 'code', text: code.join('\n') });
            continue;
        }
        const heading = HEADING.exec(line);
        if (heading !== null) {
            flush();
            add({ kind: 'heading', level: (heading[1] ?? '#').length, text: heading[2] ?? '' });
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
            const href = safeHref(match[2] ?? '');
            if (href === null) {
                plain += match[1] ?? '';
            } else {
                push({ kind: 'link', href, children: parseInline(match[1] ?? '') });
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

function BlockView({ block }: { block: Block }) {
    switch (block.kind) {
        case 'heading':
            return block.level <= 2 ? (
                <h3 className="pt-1 text-card font-medium text-ink">
                    <Inlines text={block.text} />
                </h3>
            ) : (
                <h4 className="pt-1 text-ui font-medium text-ink">
                    <Inlines text={block.text} />
                </h4>
            );
        case 'list': {
            const items = block.items.map((item) => (
                <li key={item.key}>
                    <Inlines text={item.text} />
                </li>
            ));
            return block.ordered ? (
                <ol className="flex list-decimal flex-col gap-1 pl-5">{items}</ol>
            ) : (
                <ul className="flex list-disc flex-col gap-1 pl-5">{items}</ul>
            );
        }
        case 'code':
            return (
                <pre className="rounded-box border border-line bg-surface-2 px-3 py-2 font-mono text-small break-words whitespace-pre-wrap text-ink">
                    {block.text}
                </pre>
            );
        case 'quote':
            return (
                <blockquote className="rounded-box bg-surface-2 px-3 py-2">
                    <Inlines text={block.text} />
                </blockquote>
            );
        case 'rule':
            return <hr className="border-line" />;
        case 'paragraph':
            return (
                <p className="break-words">
                    <Inlines text={block.text} />
                </p>
            );
    }
}

function Inlines({ text }: { text: string }) {
    return <>{render(parseInline(text))}</>;
}

const render = (nodes: Inline[]): ReactNode =>
    nodes.map((node) => {
        switch (node.kind) {
            case 'text':
                return <Fragment key={node.key}>{node.text}</Fragment>;
            case 'code':
                return (
                    <code
                        key={node.key}
                        className="rounded-xs bg-surface-2 px-1 font-mono text-[0.92em] text-ink"
                    >
                        {node.text}
                    </code>
                );
            case 'strong':
                return (
                    <strong key={node.key} className="font-semibold text-ink">
                        {render(node.children)}
                    </strong>
                );
            case 'em':
                return <em key={node.key}>{render(node.children)}</em>;
            case 'link':
                return (
                    <a
                        key={node.key}
                        href={node.href}
                        target="_blank"
                        rel="noreferrer noopener"
                        className="font-medium break-all text-ink underline-offset-2 hover:underline"
                    >
                        {render(node.children)}
                    </a>
                );
        }
    });
