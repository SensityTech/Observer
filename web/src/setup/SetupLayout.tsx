import { Link } from '@tanstack/react-router';
import { ShieldCheck } from 'lucide-react';
import type { ReactNode } from 'react';

import { buttonVariants } from '../components/Button';
import { PlainHttpNotice } from '../components/PlainHttpNotice';
import { useI18n } from '../i18n/I18nProvider';
import { Logo } from '../shell/Logo';
import { SETUP_STEPS, type SetupStep } from './state';
import { StepRail } from './StepRail';

/**
 * The setup on the whole screen: its head with the step and the way out, the steps down the side
 * with what each one chose, and the step itself. Narrower, the side gives way to the step's own
 * progress line.
 */
export function SetupLayout({
    current,
    chosen,
    saved,
    children,
}: {
    /** The step shown, or the first launch once the settings are saved. */
    current: SetupStep | 'launch';
    /** What each passed step chose, as the side recalls it. */
    chosen: Partial<Record<SetupStep, string>>;
    /** Whether launch settings were saved before: leaving goes back to the panel, else to them. */
    saved: boolean;
    children: ReactNode;
}) {
    const { t, format } = useI18n();
    const words = t.serverSetup;
    const index = current === 'launch' ? SETUP_STEPS.length : SETUP_STEPS.indexOf(current);
    return (
        <div className="flex min-h-dvh flex-col bg-bg text-ink">
            <a className="skip" href="#content">
                {t.app.skipToContent}
            </a>
            <header className="sticky top-0 z-(--z-rail) flex h-16 flex-none items-center gap-3.5 border-b border-line bg-bg px-7 max-md:px-4">
                <Logo />
                <span className="h-5 w-px bg-line-strong max-md:hidden" aria-hidden="true" />
                <span className="text-ui text-ink-2 max-md:hidden">{words.title}</span>
                <span className="flex-1" />
                <span className="font-mono text-meta tracking-[0.04em] text-ink-3 uppercase max-lg:hidden">
                    {current === 'launch'
                        ? words.launch.title
                        : words.stepCount(format.number(index + 1), format.number(SETUP_STEPS.length))}
                </span>
                {saved ? (
                    <Link to="/" className={buttonVariants({ tone: 'secondary', size: 'sm' })}>
                        {words.leave}
                    </Link>
                ) : (
                    <Link
                        to="/settings/$category"
                        params={{ category: 'launch' }}
                        className={buttonVariants({ tone: 'secondary', size: 'sm' })}
                    >
                        {words.later}
                    </Link>
                )}
            </header>
            <div className="flex flex-1 items-stretch">
                <aside className="flex w-75 flex-none flex-col gap-6 border-r border-line px-5 py-8 max-lg:hidden">
                    <StepRail current={index} chosen={chosen} />
                    <p className="mt-auto flex gap-2.5 rounded-box border border-line bg-bg-subtle p-3.5 text-small text-ink-3">
                        <ShieldCheck
                            size={16}
                            strokeWidth={1.75}
                            className="mt-0.5 flex-none text-success"
                            aria-hidden="true"
                        />
                        {words.promise}
                    </p>
                </aside>
                <main id="content" tabIndex={-1} className="flex min-w-0 flex-1 flex-col outline-none">
                    <div className="px-12 pt-6 empty:hidden max-md:px-4">
                        <PlainHttpNotice />
                    </div>
                    {children}
                </main>
            </div>
        </div>
    );
}
