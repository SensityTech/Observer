import type { Comparison } from '../api/generated/Comparison';
import type { NumberChange } from '../api/generated/NumberChange';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { FILTERED, changedResources, isBusy } from './words';

type Fact = { label: string; figure: string; sub: string | null; same?: boolean };

/** The comparison in six facts, each `Same` when it did not change. */
export function SummaryBand({ comparison }: { comparison: Comparison }) {
    const { t, format } = useI18n();
    const words = t.changes.summary;
    const { counts, environment, to } = comparison;
    const unknown = t.status.unknown;
    const changed = changedResources(counts);
    const same = (label: string, sub: string | null = null): Fact => ({
        label,
        figure: words.same,
        sub,
        same: true,
    });
    const number = (
        label: string,
        change: NumberChange | null,
        now: number | null,
        extra: string | null = null,
    ) =>
        change === null
            ? same(
                  label,
                  now === null ? extra : [String(now), extra].filter((part) => part !== null).join(' · '),
              )
            : {
                  label,
                  figure: change.to === null ? unknown : String(change.to),
                  sub: [words.from(change.from === null ? unknown : String(change.from)), extra]
                      .filter((part) => part !== null)
                      .join(' · '),
              };

    const resources: Fact = to.baseline
        ? { label: words.resources, figure: t.changes.baseline, sub: words.baselineSub }
        : changed === 0 && counts.checking === 0
          ? same(words.resources, words.unchanged(counts.unchanged))
          : {
                label: words.resources,
                figure: format.number(changed),
                sub: FILTERED.filter((kind) => counts[kind] > 0)
                    .map((kind) => t.changes.counted[kind](counts[kind]))
                    .join(' · '),
            };
    const whileStarting =
        comparison.changedWhileStarting > 0 ? words.whileStarting(comparison.changedWhileStarting) : null;
    const files: Fact = to.baseline
        ? { label: words.files, figure: t.changes.baseline, sub: null }
        : counts.files === 0
          ? same(words.files, whileStarting)
          : {
                label: words.files,
                figure: format.number(counts.files),
                sub: [words.filesSub, whileStarting].filter((part) => part !== null).join(' · '),
            };
    const game =
        environment.game === null
            ? null
            : t.changes.fromTo(
                  environment.game.from === null ? unknown : t.serverSetup.cfg.games[environment.game.from],
                  environment.game.to === null ? unknown : t.serverSetup.cfg.games[environment.game.to],
              );
    const cfg = environment.cfg;
    const launch = [
        ...environment.settings.map(
            (setting) => t.changes.environment.settingNames[setting.setting] ?? setting.setting,
        ),
        ...environment.arguments.map((argument) => argument.name),
    ];
    const facts: Fact[] = [
        resources,
        files,
        number(words.build, environment.build, to.build),
        number(words.gameBuild, environment.gameBuild, to.gameBuild, game),
        cfg.changed === 0
            ? same(words.cfg)
            : {
                  label: words.cfg,
                  figure: words.cfgFiles(cfg.changed),
                  sub: t.changes.lines(format.number(cfg.lines.added), format.number(cfg.lines.removed)),
              },
        launch.length === 0
            ? same(words.launch)
            : { label: words.launch, figure: words.settings(launch.length), sub: launch.join(', ') },
    ];

    return (
        <section className="card overflow-hidden" aria-label={words.label} aria-busy={isBusy(to)}>
            <dl className="grid grid-cols-2 gap-px bg-line md:grid-cols-3 lg:grid-cols-6">
                {facts.map((fact) => (
                    <div
                        key={fact.label}
                        className="flex min-w-0 flex-col gap-2 bg-surface px-4 py-4.5 max-md:p-4"
                    >
                        <dt className="eyebrow">{fact.label}</dt>
                        <dd className="flex min-w-0 flex-col gap-1">
                            <span
                                className={cn(
                                    'text-figure max-md:text-state',
                                    fact.same === true ? 'text-ink-2' : 'text-ink',
                                )}
                            >
                                {fact.figure}
                            </span>
                            {fact.sub !== null && fact.sub !== '' && (
                                <span className="text-small text-ink-3 [overflow-wrap:anywhere]">
                                    {fact.sub}
                                </span>
                            )}
                        </dd>
                    </div>
                ))}
            </dl>
        </section>
    );
}
