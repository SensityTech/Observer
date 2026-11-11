// How the Documentation page reads the integration contract: its sections, the events and exports
// among them, and where a name written in the text leads. The contract is the binary's own
// `resource/CONTRACT.md`; nothing of its text is written again here.

import type { BridgeExport } from '../api/generated/BridgeExport';
import type { ExampleFile } from '../api/generated/ExampleFile';
import type { IntegrationDocs } from '../api/generated/IntegrationDocs';
import { parseBlocks, parseInline, plainText, type Block, type Inline } from '../lib/markdown';

/** A third-level heading of the contract: an event, as a rule. */
export type DocEntry = {
    id: string;
    title: string;
    /** The bridge raises it. */
    event: boolean;
    /** When it comes, in Markdown, as the first table that lists it says. */
    when: string | null;
    blocks: Block[];
};

/** A second-level heading of the contract and what it holds. */
export type DocSection = {
    id: string;
    title: string;
    blocks: Block[];
    entries: DocEntry[];
    /** The bridge's exports this section shows called. */
    exports: BridgeExport[];
    /** The files a link of this section leads to, such as the example resource's. */
    files: ExampleFile[];
};

export type DocModel = {
    /** What the contract says before its first section. */
    lead: Block[];
    sections: DocSection[];
    /** Where a name leads when the text writes it in code: a section's or an entry's title. */
    anchors: Map<string, string>;
};

const text = (markdown: string) => plainText(parseInline(markdown));

/**
 * The address of a heading as GitHub makes it, so that the contract's own links lead to the same
 * places in the panel: `Player labels` is `player-labels`.
 */
export const anchorOf = (title: string) =>
    text(title)
        .toLowerCase()
        .replace(/[^\p{L}\p{N}\s_-]/gu, '')
        .trim()
        .replace(/\s/g, '-');

/** Where the files of a path the contract links to are shown. */
export const filesAnchor = (path: string) => `files-${anchorOf(path)}`;

const inlinesOf = (block: Block): string[] => {
    switch (block.kind) {
        case 'heading':
        case 'paragraph':
        case 'quote':
            return [block.text];
        case 'list':
            return block.items.map((item) => item.text);
        case 'table':
            return [...block.head, ...block.rows.flatMap((row) => row.cells)].map((cell) => cell.text);
        case 'code':
        case 'rule':
            return [];
    }
};

/** Everything a block says, its code included. */
const said = (block: Block) => (block.kind === 'code' ? block.text : inlinesOf(block).join('\n'));

const localTargets = (nodes: Inline[]): string[] =>
    nodes.flatMap((node) =>
        node.kind === 'local'
            ? [node.target, ...localTargets(node.children)]
            : 'children' in node
              ? localTargets(node.children)
              : [],
    );

export function buildDocs({
    contract,
    events,
    exports,
    example,
}: Pick<IntegrationDocs, 'contract' | 'events' | 'exports' | 'example'>): DocModel {
    const blocks = parseBlocks(contract);
    const lead: Block[] = [];
    const sections: DocSection[] = [];
    const taken = new Map<string, number>();
    // A title met again takes a number, as on GitHub.
    const unique = (id: string) => {
        const seen = taken.get(id) ?? 0;
        taken.set(id, seen + 1);
        return seen === 0 ? id : `${id}-${seen}`;
    };
    const when = new Map<string, string>();
    for (const block of blocks) {
        if (block.kind !== 'table') {
            continue;
        }
        for (const { cells } of block.rows) {
            const name = text(cells[0]?.text ?? '');
            if (events.includes(name) && !when.has(name)) {
                when.set(name, cells[1]?.text ?? '');
            }
        }
    }

    for (const block of blocks) {
        if (block.kind === 'heading' && block.level === 1) {
            continue;
        }
        if (block.kind === 'heading' && block.level === 2) {
            const title = text(block.text);
            sections.push({
                id: unique(anchorOf(title)),
                title,
                blocks: [],
                entries: [],
                exports: [],
                files: [],
            });
            continue;
        }
        const section = sections.at(-1);
        if (section === undefined) {
            lead.push(block);
        } else if (block.kind === 'heading' && block.level === 3) {
            const title = text(block.text);
            section.entries.push({
                id: unique(anchorOf(title)),
                title,
                event: events.includes(title),
                when: when.get(title) ?? null,
                blocks: [],
            });
        } else {
            (section.entries.at(-1) ?? section).blocks.push(block);
        }
    }

    const anchors = new Map<string, string>();
    const placed = new Set<string>();
    for (const section of sections) {
        const all = [...section.blocks, ...section.entries.flatMap((entry) => entry.blocks)];
        const whole = all.map(said).join('\n');
        // An export belongs to the first section that shows it called.
        section.exports = exports.filter(({ name }) => !placed.has(name) && whole.includes(`]:${name}(`));
        for (const { name } of section.exports) {
            placed.add(name);
        }
        const linked = new Set(all.flatMap(inlinesOf).flatMap((inline) => localTargets(parseInline(inline))));
        section.files = example.filter((file) =>
            [...linked].some((path) => !path.startsWith('#') && file.name.startsWith(`${path}/`)),
        );
        anchors.set(section.title, section.id);
        for (const entry of section.entries) {
            anchors.set(entry.title, entry.id);
        }
    }
    return { lead, sections, anchors };
}

/** The fields of an event, as the first table of its section names them. */
export const fieldsOf = (entry: DocEntry): string[] => {
    const table = entry.blocks.find((block) => block.kind === 'table');
    return table?.kind === 'table' ? table.rows.map((row) => text(row.cells[0]?.text ?? '')) : [];
};

/** The widest a line of a handler runs, so that it fits its block without scrolling. */
const HANDLER_WIDTH = 72;
const COMMENT = '    --';

const folded = (words: string[]): string[] => {
    const lines: string[] = [];
    let line = COMMENT;
    for (const word of words) {
        if (line !== COMMENT && `${line} ${word}`.length > HANDLER_WIDTH) {
            lines.push(line);
            line = COMMENT;
        }
        line += ` ${word}`;
    }
    return line === COMMENT ? lines : [...lines, line];
};

/**
 * A handler to copy for an event: its fields as its table names them, then what the example
 * resource says goes there, in its own words.
 */
export const handlerOf = (entry: DocEntry, example: string): string => {
    const fields = fieldsOf(entry).map((field) => `payload.${field}`);
    const lines = folded(fields.map((field, index) => (index < fields.length - 1 ? `${field},` : field)));
    const start = example.indexOf(`AddEventHandler('${entry.title}'`);
    if (start >= 0) {
        const end = example.indexOf('\nend)', start);
        for (const row of example.slice(start, end < 0 ? undefined : end).split('\n')) {
            const comment = /^\s*--\s*(.*)$/.exec(row);
            if (comment !== null) {
                lines.push(...folded((comment[1] ?? '').split(' ').filter(Boolean)));
            }
        }
    }
    return [`AddEventHandler('${entry.title}', function(payload)`, ...lines, 'end)'].join('\n');
};

export type DocMatch = {
    /** Where it is on the page. */
    id: string;
    label: string;
    kind: 'section' | 'event' | 'export' | 'txadmin';
};

const TXADMIN_EVENT = /\btxAdmin:events:\w+/g;

/**
 * What the search finds in the documentation: its sections, each event, each export, and txAdmin's
 * events by their own names.
 */
export const docIndex = (model: DocModel): DocMatch[] =>
    model.sections.flatMap((section) => {
        const all = [...section.blocks, ...section.entries.flatMap((entry) => entry.blocks)];
        const txadmin = new Set(all.flatMap((block) => said(block).match(TXADMIN_EVENT) ?? []));
        return [
            { id: section.id, label: section.title, kind: 'section' as const },
            ...section.entries.map((entry) => ({
                id: entry.id,
                label: entry.title,
                kind: entry.event ? ('event' as const) : ('section' as const),
            })),
            ...section.exports.map(({ name }) => ({ id: section.id, label: name, kind: 'export' as const })),
            ...[...txadmin].map((name) => ({ id: section.id, label: name, kind: 'txadmin' as const })),
        ];
    });
