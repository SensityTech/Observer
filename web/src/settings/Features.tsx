import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Lock } from 'lucide-react';
import { useId, useState } from 'react';

import { featuresQuery, switchFeature } from '../api/features';
import type { Cadence } from '../api/generated/Cadence';
import type { Feature } from '../api/generated/Feature';
import type { FeatureView } from '../api/generated/FeatureView';
import { useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf, SWITCH_OF } from './catalog';
import { CategoryHead } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };

const cadenceOf = (t: Messages, duration: (seconds: number) => string, cadence: Cadence) =>
    cadence.kind === 'every'
        ? t.settings.features.cadences.every(duration(cadence.seconds))
        : t.settings.features.cadences[cadence.kind];

/** Its state in a word, with the LED it goes with: on, off, or held back by the host or another feature. */
const stateOf = (t: Messages, view: FeatureView): { led: string; text: string } => {
    const words = t.settings.features.states;
    if (view.held === 'host') {
        return { led: 'led-down', text: words.locked };
    }
    if (!view.enabled) {
        return { led: 'led-off', text: words.off };
    }
    if (view.running) {
        return { led: 'led-ok', text: words.on };
    }
    if (view.held === 'notConfigured') {
        return { led: 'led-warning', text: words.notConfigured };
    }
    if (view.held === 'requires') {
        return {
            led: 'led-warning',
            text: words.requires(view.requires.map((feature) => t.settings.items[SWITCH_OF[feature]].label)),
        };
    }
    return {
        led: 'led-pending',
        text: view.turnOn === 'nextServerStart' ? words.nextServerStart : words.observerRestart,
    };
};

/**
 * Observer's optional features, each with what it does, what it costs and a switch: what the
 * owner turns off runs nothing. Reading them takes View settings; switching one is the owner's.
 */
export function FeatureSettings() {
    const { t } = useI18n();
    const words = t.settings.features;
    const me = useMe();
    const client = useQueryClient();
    const features = useQuery(featuresQuery);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const toggle = useMutation({
        mutationFn: ({ feature, enabled }: { feature: Feature; enabled: boolean }) =>
            switchFeature(feature, enabled),
        onMutate: () => setOutcome(null),
        onSuccess: (list, { feature, enabled }) => {
            client.setQueryData(featuresQuery.queryKey, list);
            const name = t.settings.items[SWITCH_OF[feature]].label;
            const view = list.features.find((each) => each.feature === feature);
            setOutcome({
                tone: 'success',
                message: !enabled
                    ? words.turnedOff(name)
                    : view === undefined || view.running
                      ? words.turnedOn(name)
                      : words.turnedOnLater(name, stateOf(t, view).text),
            });
        },
        onError: (error, { feature }) =>
            setOutcome({
                tone: 'danger',
                message: `${t.settings.items[SWITCH_OF[feature]].label}: ${errorMessage(t, error)}`,
            }),
    });

    return (
        <>
            <CategoryHead category="features" />
            <p className="flex items-start gap-2.5 rounded-box border border-line bg-surface-2 px-3.5 py-2.5 text-small text-ink-2">
                <Lock
                    size={14}
                    strokeWidth={1.5}
                    className="mt-0.5 flex-none text-ink-3"
                    aria-hidden="true"
                />
                {words.locked}
            </p>
            {!me.isOwner && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            {features.isError && features.data === undefined && (
                <Notice tone="danger">
                    {words.loadFailed} {errorMessage(t, features.error)}
                </Notice>
            )}
            {features.isPending && (
                <div className="grid grid-cols-2 gap-3 max-lg:grid-cols-1" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <div className="skeleton h-40 rounded-card" />
                    <div className="skeleton h-40 rounded-card" />
                </div>
            )}
            {features.data !== undefined && (
                <div className="grid grid-cols-2 gap-3 max-lg:grid-cols-1">
                    {features.data.features.map((view) => (
                        <FeatureCard
                            key={view.feature}
                            view={view}
                            canSwitch={me.isOwner}
                            pending={toggle.isPending && toggle.variables.feature === view.feature}
                            busy={toggle.isPending}
                            onSwitch={(enabled) => toggle.mutate({ feature: view.feature, enabled })}
                        />
                    ))}
                </div>
            )}
            <section className="card bg-bg-subtle px-5 py-3.5 max-md:px-4">
                <p className="text-small text-ink-2">
                    <b className="font-semibold text-ink">{words.alwaysOn}</b> · {words.alwaysOnBody}
                </p>
            </section>
        </>
    );
}

function FeatureCard({
    view,
    canSwitch,
    pending,
    busy,
    onSwitch,
}: {
    view: FeatureView;
    canSwitch: boolean;
    pending: boolean;
    busy: boolean;
    onSwitch: (enabled: boolean) => void;
}) {
    const { t, format } = useI18n();
    const words = t.settings.features;
    const id = useId();
    const setting = SWITCH_OF[view.feature];
    const { label, about } = t.settings.items[setting];
    const state = stateOf(t, view);
    const { reach } = view;
    const costs = [
        ...reach.listens.map((address) => words.listens(address)),
        ...new Set(reach.tasks.map(({ cadence }) => cadenceOf(t, format.duration, cadence))),
        ...reach.files.map(({ path, bytes }) =>
            words.file(path, bytes === null ? null : format.bytes(bytes)),
        ),
        ...(words.serverFolder[reach.serverFolder] === null ? [] : [words.serverFolder[reach.serverFolder]]),
    ];

    return (
        <section
            id={anchorOf(setting)}
            className="card flex flex-col gap-2.5 px-4.5 py-4 max-md:px-4"
            aria-labelledby={`${id}-name`}
        >
            <div className="flex items-start gap-3">
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <h3 id={`${id}-name`} className="text-body font-semibold text-ink">
                        {label}
                    </h3>
                    <p id={`${id}-about`} className="text-small text-pretty text-ink-2">
                        {about}
                    </p>
                </div>
                <Switch
                    checked={view.enabled}
                    disabled={!canSwitch || busy || view.held === 'host'}
                    aria-labelledby={`${id}-name`}
                    aria-describedby={`${id}-about ${id}-state`}
                    aria-busy={pending || undefined}
                    onChange={(event) => onSwitch(event.target.checked)}
                />
            </div>
            <ul className="flex flex-wrap gap-1.5">
                {costs.map((cost) => (
                    <li
                        key={cost}
                        className="inline-flex h-5.5 items-center rounded-pill border border-line bg-surface-2 px-2 font-mono text-meta leading-none text-ink-2"
                    >
                        {cost}
                    </li>
                ))}
            </ul>
            {reach.whileOff.length > 0 && !view.enabled && (
                <p className="text-small text-ink-3">
                    {words.whileOff(reach.whileOff.map(({ task }) => words.tasks[task]))}
                </p>
            )}
            <p className="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-line pt-2.5 text-small">
                <span className={cn('led', state.led)} aria-hidden="true" />
                <span id={`${id}-state`} className={view.enabled ? 'text-ink-2' : 'text-ink-3'}>
                    {state.text}
                </span>
                <span className="ml-auto text-ink-3">
                    {view.enabled ? words.turnOff[view.turnOff] : words.turnOn[view.turnOn]}
                </span>
            </p>
        </section>
    );
}
