import type { CfgEndpointLine } from '../api/generated/CfgEndpointLine';
import type { CfgOutline } from '../api/generated/CfgOutline';
import type { CfgSetting } from '../api/generated/CfgSetting';

/** The convars the guided view has a field for; the server's outline reads the same names alone on a line. */
export const FIELDS = {
    sv_hostname: { added: 'sv_hostname', number: false },
    sv_projectName: { added: 'sets sv_projectName', number: false },
    sv_projectDesc: { added: 'sets sv_projectDesc', number: false },
    tags: { added: 'sets tags', number: false },
    locale: { added: 'sets locale', number: false },
    sv_maxclients: { added: 'sv_maxclients', number: true },
    sv_enforceGameBuild: { added: 'sv_enforceGameBuild', number: true },
    sv_licenseKey: { added: 'sv_licenseKey', number: false },
    steam_webApiKey: { added: 'set steam_webApiKey', number: false },
} as const satisfies Record<string, { added: string; number: boolean }>;

export type FieldId = keyof typeof FIELDS;

const FIELD_IDS = Object.keys(FIELDS) as FieldId[];

/** What the guided view changed, on top of a text and its outline. */
export type Guided = {
    values: Partial<Record<FieldId, string>>;
    /** The game address, written to the TCP and the UDP endpoint alike. */
    endpoint: string | null;
    /** Whether a resource line starts its resource, by its line in the text. */
    started: Record<number, boolean>;
    /** Resources to start after those the file already starts. */
    added: string[];
};

export const UNTOUCHED: Guided = { values: {}, endpoint: null, started: {}, added: [] };

export const touched = (guided: Guided) =>
    Object.keys(guided.values).length > 0 ||
    guided.endpoint !== null ||
    Object.keys(guided.started).length > 0 ||
    guided.added.length > 0;

const same = (left: string, right: string) => left.toLowerCase() === right.toLowerCase();

/** The line FXServer keeps the convar from: the last one that sets it. */
export const settingOf = (outline: CfgOutline, field: FieldId): CfgSetting | null =>
    outline.settings.findLast((setting) => same(setting.name, field)) ?? null;

/** How many lines set the convar: more than one, and only the last counts. */
export const settingsOf = (outline: CfgOutline, field: FieldId) =>
    outline.settings.filter((setting) => same(setting.name, field)).length;

/**
 * The one TCP and the one UDP endpoint of a cfg that opens the same address on both, which one
 * field then edits; null for any other set of endpoints, left to the file view.
 */
export const pairedEndpoints = (outline: CfgOutline): [CfgEndpointLine, CfgEndpointLine] | null => {
    const [first, second, ...more] = outline.endpoints;
    if (first === undefined || second === undefined || more.length > 0) {
        return null;
    }
    return first.protocol !== second.protocol && first.address === second.address ? [first, second] : null;
};

/**
 * A value as a cfg holds it: between quotes, a backslash doubled. FXServer ends a command at a `;`
 * that follows a quote, escaped or not, so a value with a quote or a line break is not written.
 */
export const writable = (value: string) => !/["\r\n]/u.test(value);

const quoted = (value: string) => `"${value.replaceAll('\\', '\\\\')}"`;

const written = (value: string, number: boolean) => (number && /^\d+$/u.test(value) ? value : quoted(value));

/** A resource or a category as a cfg names one: no space, quote, `;` or comment sign. */
export const resourceName = (name: string) => /^[^\s";#]+$/u.test(name) && !name.includes('//');

/** A line that starts a resource, commented out or put back as it was. */
export const toggled = (line: string, start: boolean) => {
    const [, indent = '', rest = ''] = /^(\s*)(.*)$/su.exec(line) ?? [];
    if (!start) {
        return `${indent}#${rest}`;
    }
    return indent + rest.replace(/^(?:#|\/\/)\s*/u, '');
};

/**
 * The text with the guided changes written into it: each value in place of the one its line held,
 * the lines of the resources turned off commented out, and a line added for what the file did not
 * set or start. Every other character stays as it was.
 */
export const applyGuided = (text: string, outline: CfgOutline, guided: Guided) => {
    const lines = text.split('\n');
    /** Replacements within lines, applied from the end of each line. */
    const patches: { line: number; start: number; end: number; text: string }[] = [];
    /** Lines added after a line of the text, 0 for the top. */
    const insertions: { after: number; text: string }[] = [];

    const fieldLines = [
        ...outline.settings.filter((setting) => FIELD_IDS.some((field) => same(setting.name, field))),
        ...outline.endpoints,
    ].map((entry) => entry.line);
    const lastField = Math.max(0, ...fieldLines);

    for (const field of FIELD_IDS) {
        const value = guided.values[field];
        if (value === undefined || !writable(value)) {
            continue;
        }
        const { added, number } = FIELDS[field];
        const setting = settingOf(outline, field);
        if (setting === null) {
            insertions.push({ after: lastField, text: `${added} ${written(value, number)}` });
        } else {
            patches.push({
                line: setting.line,
                start: setting.start,
                end: setting.end,
                text: written(value, number),
            });
        }
    }
    const endpoints = pairedEndpoints(outline);
    if (guided.endpoint !== null && endpoints !== null && writable(guided.endpoint)) {
        for (const endpoint of endpoints) {
            patches.push({
                line: endpoint.line,
                start: endpoint.start,
                end: endpoint.end,
                text: quoted(guided.endpoint),
            });
        }
    }

    for (const patch of patches.toSorted((left, right) => right.start - left.start)) {
        const line = lines[patch.line - 1];
        if (line !== undefined) {
            lines[patch.line - 1] = line.slice(0, patch.start) + patch.text + line.slice(patch.end);
        }
    }
    for (const resource of outline.resources) {
        const start = guided.started[resource.line];
        const line = lines[resource.line - 1];
        if (start !== undefined && start !== resource.enabled && resource.alone && line !== undefined) {
            lines[resource.line - 1] = toggled(line, start);
        }
    }

    // After the last line that starts something, or at the end of the file, before its final line end.
    const end = lines.at(-1) === '' ? lines.length - 1 : lines.length;
    const lastResource = Math.max(0, ...outline.resources.map((resource) => resource.line)) || end;
    for (const name of guided.added) {
        if (resourceName(name)) {
            insertions.push({ after: lastResource, text: `ensure ${name}` });
        }
    }
    // From the bottom up, so that a line added does not move the place of the next; those added
    // at one place keep their order.
    for (const after of [...new Set(insertions.map((insertion) => insertion.after))].toSorted(
        (left, right) => right - left,
    )) {
        const added = insertions
            .filter((insertion) => insertion.after === after)
            .map((insertion) => insertion.text);
        lines.splice(after, 0, ...added);
    }
    return lines.join('\n');
};
