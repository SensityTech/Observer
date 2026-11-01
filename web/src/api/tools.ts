import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { ToolFixRequest } from './generated/ToolFixRequest';
import type { ToolRun } from './generated/ToolRun';
import type { ToolSettingsChange } from './generated/ToolSettingsChange';
import type { ToolsView } from './generated/ToolsView';
import type { ToolUndoRequest } from './generated/ToolUndoRequest';
import type { ToolUndoScope } from './generated/ToolUndoScope';

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

/** A run whose fixes were written: its report says what it did, no longer what to do. */
export const applied = (run: ToolRun) => run.mode === 'fix' && (run.summary?.filesChanged ?? 0) > 0;

/** The client's way of writing an archetype's hash: `F032591F`. */
export const hashText = (hash: number) => hash.toString(16).toUpperCase().padStart(8, '0');

/** The same scope, the same key: which undo button is waiting. */
export const scopeKey = (scope: ToolUndoScope) =>
    typeof scope === 'string'
        ? scope
        : 'run' in scope
          ? `run ${scope.run}`
          : `changes ${scope.changes.join(' ')}`;
