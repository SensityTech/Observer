import { Clock, OctagonX, Package, RotateCw, Users, Zap, type LucideIcon } from 'lucide-react';
import { useEffect, useRef } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { ditherField } from './dither';

type Sample = keyof Messages['signIn']['stage']['samples'];

// What each sample event looks like; its words are in the catalog. The first was a user's doing,
// the others Observer's own (audit actor `system`), shown with its mark.
const SAMPLES: { sample: Sample; byUser: boolean; icon: LucideIcon; tint: string }[] = [
    { sample: 'user', byUser: true, icon: Users, tint: 'tint-neutral' },
    { sample: 'crash', byUser: false, icon: OctagonX, tint: 'tint-danger' },
    { sample: 'hitch', byUser: false, icon: Zap, tint: 'tint-warning' },
    { sample: 'build', byUser: false, icon: Package, tint: 'tint-success' },
    { sample: 'restart', byUser: false, icon: RotateCw, tint: 'tint-accent' },
];

/**
 * The always dark stage beside the sign-in forms. Everything on it is sample data, labelled as
 * such: nothing about the server shows before signing in.
 */
export function Stage() {
    const { t } = useI18n();
    const stage = t.signIn.stage;
    const field = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const host = field.current;
        return host === null ? undefined : ditherField(host);
    }, []);

    return (
        <div className="signin-stage">
            <section data-theme="dark" className="signin-panel" aria-labelledby="signin-stage-title">
                <div className="signin-stage-head">
                    <span className="signin-eyebrow" id="signin-stage-title">
                        {stage.title}
                    </span>
                    <span className="signin-live">
                        <span className="led led-live" aria-hidden="true" />
                        {stage.live}
                    </span>
                </div>

                <div className="signin-iris" aria-hidden="true">
                    <div className="signin-dither" ref={field}>
                        <div className="signin-dither-still" />
                    </div>
                    <span className="signin-core">
                        <svg viewBox="0 0 24 24">
                            <circle
                                cx="12"
                                cy="12"
                                r="6.25"
                                fill="none"
                                stroke="currentColor"
                                strokeWidth="2"
                            />
                            <circle cx="12" cy="12" r="2.25" fill="currentColor" />
                        </svg>
                    </span>
                    <span className="signin-tag signin-tag-tick tint-success">
                        <span className="signin-swatch" />
                        {stage.tick.name} · <b>{stage.tick.value}</b>
                    </span>
                    <span className="signin-tag signin-tag-players tint-accent">
                        <span className="signin-swatch" />
                        {stage.players.name} · <b>{stage.players.value}</b>
                    </span>
                </div>

                <div className="signin-veil" aria-hidden="true" />

                <div className="signin-fact-block">
                    <span className="signin-marker">{stage.marker}</span>
                    <p className="signin-fact">
                        {stage.fact} <span className="signin-tone">{stage.factTone}</span>
                    </p>
                    <p className="signin-fact-body">
                        {stage.factBody.before}
                        <code className="signin-code">{stage.factBody.code}</code>
                        {stage.factBody.after}
                    </p>
                </div>

                <div className="signin-events">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                        <span className="signin-eyebrow">{stage.events}</span>
                        <span className="signin-xs text-ink-3">{stage.sample}</span>
                    </div>
                    <section className="signin-ticker" aria-label={stage.ticker}>
                        <div className="signin-ticker-track">
                            <Ticks />
                            <Ticks copy />
                        </div>
                    </section>
                </div>
            </section>
        </div>
    );
}

/** One round of the ticker; the copy that makes the loop seamless is hidden from assistive technology. */
function Ticks({ copy = false }: { copy?: boolean }) {
    const { t } = useI18n();
    const stage = t.signIn.stage;

    return SAMPLES.map(({ sample, byUser, icon: Icon, tint }) => {
        const words = stage.samples[sample];
        return (
            <div key={sample} className="signin-tick" aria-hidden={copy || undefined}>
                <span className="signin-initials">
                    {byUser ? (
                        stage.ownerInitials
                    ) : (
                        <>
                            <svg viewBox="0 0 24 24" aria-hidden="true">
                                <circle
                                    cx="12"
                                    cy="12"
                                    r="6.25"
                                    fill="none"
                                    stroke="var(--ink)"
                                    strokeWidth="2"
                                />
                                <circle cx="12" cy="12" r="2.25" fill="var(--accent)" />
                            </svg>
                            <span className="sr-only">{stage.observer}</span>
                        </>
                    )}
                </span>
                <span className="signin-tick-main">
                    <span className="signin-tick-title">{words.title}</span>
                    <span className="signin-tick-detail">{words.detail}</span>
                </span>
                <span className="signin-tick-side">
                    <span className="signin-tick-figure">{words.figure}</span>
                    <span className="signin-tick-time">
                        <Clock size={12} strokeWidth={1.5} aria-hidden="true" />
                        {words.time}
                    </span>
                </span>
                <span className={`signin-thumb ${tint}`}>
                    <Icon size={16} strokeWidth={1.5} aria-hidden="true" />
                </span>
            </div>
        );
    });
}
