import { describe, expect, it } from 'vitest';

import { parseBlocks, parseInline, type Inline } from '../lib/markdown';
import { integrationDocs } from './fixtures';
import { anchorOf, buildDocs, docIndex, fieldsOf, filesAnchor, handlerOf } from './model';

const docs = integrationDocs();
const model = buildDocs(docs);
const entries = model.sections.flatMap((section) => section.entries);
const script = docs.example.find((file) => file.name.endsWith('server.lua'))?.text ?? '';

describe('anchorOf', () => {
    it('makes the address GitHub makes of a heading', () => {
        expect(anchorOf('Player labels')).toBe('player-labels');
        expect(anchorOf("txAdmin's events")).toBe('txadmins-events');
        expect(anchorOf('`observer:restartScheduled`')).toBe('observerrestartscheduled');
        expect(anchorOf('Not in version 4')).toBe('not-in-version-4');
    });
});

describe('buildDocs, on the contract as the repository holds it', () => {
    it('gives every heading of the contract a place, each with its own address', () => {
        const headings = parseBlocks(docs.contract).filter((block) => block.kind === 'heading');
        const ids = [...model.sections.map((section) => section.id), ...entries.map((entry) => entry.id)];

        expect(model.sections).toHaveLength(headings.filter((heading) => heading.level === 2).length);
        expect(entries).toHaveLength(headings.filter((heading) => heading.level === 3).length);
        expect(new Set(ids).size).toBe(ids.length);
        expect(model.lead).not.toHaveLength(0);
    });

    it('knows every event the bridge raises, when it comes and what it carries', () => {
        expect(docs.events).not.toHaveLength(0);
        for (const event of docs.events) {
            const entry = entries.find((candidate) => candidate.title === event);

            expect(entry, event).toMatchObject({ event: true });
            expect(entry?.when, event).toBeTruthy();
            expect(entry === undefined ? [] : fieldsOf(entry), event).not.toHaveLength(0);
            expect(model.anchors.get(event), event).toBe(entry?.id);
        }
    });

    it('places every export of the bridge in the one section that shows it called', () => {
        expect(docs.exports).not.toHaveLength(0);
        for (const { name } of docs.exports) {
            const holders = model.sections.filter((section) =>
                section.exports.some((placed) => placed.name === name),
            );

            expect(holders, name).toHaveLength(1);
        }
    });

    it('leads every link of the contract somewhere on the page', () => {
        const locals = (nodes: Inline[]): string[] =>
            nodes.flatMap((node) =>
                node.kind === 'local' ? [node.target] : 'children' in node ? locals(node.children) : [],
            );
        const targets = parseBlocks(docs.contract).flatMap((block) => {
            const texts =
                block.kind === 'paragraph'
                    ? [block.text]
                    : block.kind === 'list'
                      ? block.items.map((item) => item.text)
                      : block.kind === 'table'
                        ? block.rows.flatMap((row) => row.cells.map((cell) => cell.text))
                        : [];
            return texts.flatMap((text) => locals(parseInline(text)));
        });
        const ids = new Set([
            ...model.sections.map((section) => section.id),
            ...entries.map((entry) => entry.id),
        ]);
        const shown = model.sections.flatMap((section) => section.files.map((file) => file.name));

        expect(targets).not.toHaveLength(0);
        for (const target of targets) {
            if (target.startsWith('#')) {
                expect(ids.has(target.slice(1)), target).toBe(true);
            } else {
                expect(
                    shown.some((name) => name.startsWith(`${target}/`)),
                    target,
                ).toBe(true);
            }
        }
        expect(filesAnchor('observer_example')).toBe('files-observer_example');
    });
});

describe('handlerOf', () => {
    it('gives each event a handler that names its fields and says what the example says', () => {
        for (const entry of entries.filter((candidate) => candidate.event)) {
            const handler = handlerOf(entry, script);
            const lines = handler.split('\n');

            expect(lines[0], entry.title).toBe(`AddEventHandler('${entry.title}', function(payload)`);
            expect(lines.at(-1), entry.title).toBe('end)');
            for (const field of fieldsOf(entry)) {
                expect(handler, entry.title).toContain(`payload.${field}`);
            }
            // The example's own comment for this event, folded.
            expect(lines.length, entry.title).toBeGreaterThan(3);
            for (const line of lines.slice(1, -1)) {
                expect(line.startsWith('    -- '), line).toBe(true);
                expect(line.length, line).toBeLessThanOrEqual(72);
            }
        }
    });

    it('stands without the example', () => {
        const entry = entries.find((candidate) => candidate.title === 'observer:shuttingDown');

        expect(entry && handlerOf(entry, '')).toBe(
            [
                "AddEventHandler('observer:shuttingDown', function(payload)",
                '    -- payload.delayMs, payload.reason, payload.by',
                'end)',
            ].join('\n'),
        );
    });
});

describe('docIndex', () => {
    it('lists the sections, the events, the exports and the names txAdmin gives its events', () => {
        const index = docIndex(model);
        const of = (kind: string) => index.filter((match) => match.kind === kind).map((match) => match.label);

        expect(of('section')).toContain('Player labels');
        expect(of('event')).toEqual(docs.events);
        expect(of('export').toSorted()).toEqual(docs.exports.map((item) => item.name).toSorted());
        expect(of('txadmin')).toContain('txAdmin:events:serverShuttingDown');
        expect(index.find((match) => match.label === 'setPlayerLabel')?.id).toBe('player-labels');
    });
});
