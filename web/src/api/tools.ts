import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { ToolChange } from './generated/ToolChange';
import type { ToolFixRequest } from './generated/ToolFixRequest';
import type { ToolRun } from './generated/ToolRun';
import type { ToolSettingsChange } from './generated/ToolSettingsChange';
import type { ToolsView } from './generated/ToolsView';
import type { ToolUndoRequest } from './generated/ToolUndoRequest';
import type { ToolUndoScope } from './generated/ToolUndoScope';
import type { ToolView } from './generated/ToolView';

const PATH = '/tools/duplicate-archetypes';

/** Each tool with its settings, its latest runs and the files it changed. */
export const toolsQuery = queryOptions({
    queryKey: ['tools'],
    queryFn: ({ signal }) => getJson<ToolsView>('/tools', signal),
});

/** Reads the server folder and records the report. Writes nothing; FXServer may run. */
export const scanArchetypes = () => send<ToolRun>('POST', `${PATH}/scan`);

/**
 * Applies the plan whose digest the report gave, with FXServer stopped. Refused with
 * `plan_changed`, the fresh run in `params.run`, when a fresh scan plans something else.
 */
export const fixArchetypes = (plan: string) =>
    send<ToolRun>('POST', `${PATH}/fix`, { plan } satisfies ToolFixRequest);

/**
 * Keeps the plan for the next start of FXServer, which writes it before FXServer starts if a
 * fresh scan plans exactly that. Refused with `plan_changed` when the plan is not the latest report's.
 */
export const stageArchetypes = (plan: string) =>
    send<ToolsView>('POST', `${PATH}/stage`, { plan } satisfies ToolFixRequest);

/** Drops the fix staged for the next start. */
export const unstageArchetypes = () => send<ToolsView>('DELETE', `${PATH}/stage`);

/** Puts back what the scope names, with FXServer stopped. */
export const undoArchetypes = (scope: ToolUndoScope) =>
    send<ToolRun>('POST', `${PATH}/undo`, { scope } satisfies ToolUndoRequest);

/** Refused with `conflict` when the settings changed since the version the change was made from. */
export const saveArchetypeSettings = (change: ToolSettingsChange) =>
    send<ToolsView>('PUT', `${PATH}/settings`, change);

/** The fresh run a `plan_changed` refusal hands back, when it holds one. */
export const freshRun = (params: Record<string, unknown>): ToolRun | null => {
    const run = params.run;
    return typeof run === 'object' &&
        run !== null &&
        'id' in run &&
        typeof run.id === 'number' &&
        'tool' in run &&
        typeof run.tool === 'string'
        ? (run as ToolRun)
        : null;
};

/**
 * The view with `run` as its tool's latest: the report shown stays the latest one made, which a run
 * without its own (an undo) leaves to the run before.
 */
export const withRun = (view: ToolsView, run: ToolRun): ToolsView => ({
    tools: view.tools.map((tool) => {
        if (tool.tool !== run.tool || tool.lastRun?.id === run.id) {
            return tool;
        }
        const lastReport =
            run.report !== null ? null : tool.lastRun?.report != null ? tool.lastRun : tool.lastReport;
        return { ...tool, lastRun: run, lastReport };
    }),
});

/**
 * A fix run whose removals some file still holds: its report says what it did, no longer what to
 * do. One that an undo or a heal put everything back from says what to do again.
 */
export const applied = (run: ToolRun, open: ToolChange[]) =>
    run.mode === 'fix' && open.some((change) => change.removals.some((removal) => removal.run === run.id));

/**
 * The groups a fix run fixed whose copies are all still taken out, by hash: not those it never got
 * to, nor those an undo, a heal or someone else's hand put back since.
 */
export const fixedGroups = (run: ToolRun | null, open: ToolChange[]): ReadonlySet<number> => {
    if (run === null || run.mode !== 'fix' || run.report === null) {
        return new Set();
    }
    const out = new Set<string>();
    for (const change of open) {
        for (const removal of change.removals) {
            if (removal.run === run.id) {
                out.add(`${removal.hash} ${change.path}`);
            }
        }
    }
    return new Set(
        run.report.groups
            .filter(
                (group) =>
                    group.action.kind === 'fix' &&
                    group.action.remove.every((at) => {
                        const copy = group.copies[at];
                        return copy !== undefined && out.has(`${group.hash} ${copy.path}`);
                    }),
            )
            .map((group) => group.hash),
    );
};

/**
 * Whether files were written since the report was made, which it does not know of. Every scan and
 * fix makes a report of its own, so a write after the report's run is an undo or a heal that put
 * entries back.
 */
export const outdated = (run: ToolRun, view: ToolView) => {
    const made = Date.parse(run.finishedAt ?? run.startedAt);
    const after = (at: string | null) => at !== null && Date.parse(at) > made;
    return (
        view.open.some((change) => after(change.changedAt)) ||
        view.closed.some(
            (change) =>
                (change.closeReason === 'healed' || change.closeReason === 'undone') &&
                after(change.closedAt),
        )
    );
};

/** The client's way of writing an archetype's hash: `F032591F`. */
export const hashText = (hash: number) => hash.toString(16).toUpperCase().padStart(8, '0');

/** The same scope, the same key: which undo button is waiting. */
export const scopeKey = (scope: ToolUndoScope) =>
    typeof scope === 'string'
        ? scope
        : 'run' in scope
          ? `run ${scope.run}`
          : `changes ${scope.changes.join(' ')}`;
