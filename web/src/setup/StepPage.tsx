import type { ReactNode, Ref } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { SETUP_STEPS, type SetupStep } from './state';

/**
 * One step of the setup: its question as the page's title, what it is about, its content, and its
 * buttons in a bar that stays at the foot of the screen.
 */
export function StepPage({
    step,
    titleRef,
    title,
    lead,
    banner,
    actions,
    children,
}: {
    step: SetupStep;
    /** The keyboard comes to the title when the step changes. */
    titleRef: Ref<HTMLHeadingElement>;
    title: string;
    lead?: ReactNode;
    /** What came of the last action, above the step. */
    banner?: ReactNode;
    actions: ReactNode;
    children: ReactNode;
}) {
    const index = SETUP_STEPS.indexOf(step);
    return (
        <>
            <div className="flex flex-1 flex-col gap-7 px-12 pt-10 pb-10 max-md:gap-5 max-md:px-4 max-md:pt-6">
                <Progress current={index} />
                <div className="flex max-w-200 flex-col gap-2.5">
                    <h1
                        ref={titleRef}
                        tabIndex={-1}
                        className="text-[30px] leading-tight font-semibold tracking-[-0.02em] text-balance outline-none max-md:text-[24px]"
                    >
                        {title}
                    </h1>
                    {lead !== undefined && (
                        <p className="max-w-170 text-[15px] leading-relaxed text-ink-2">{lead}</p>
                    )}
                </div>
                {banner}
                {children}
            </div>
            <div className="sticky bottom-0 z-(--z-dock) flex-none border-t border-line bg-bg px-12 py-4 max-md:px-4 max-md:py-3">
                {actions}
            </div>
        </>
    );
}

/** Where the side is hidden: the steps as a bar, the passed ones filled. */
function Progress({ current }: { current: number }) {
    const { t } = useI18n();
    const words = t.serverSetup;
    const step = SETUP_STEPS[current] ?? 'folder';
    return (
        <div className="flex flex-col gap-2 lg:hidden">
            <p className="text-ui font-medium text-ink-2">
                {words.stepOf(current + 1, SETUP_STEPS.length, words.stepNames[step])}
            </p>
            <div className="grid grid-cols-4 gap-1" aria-hidden="true">
                {SETUP_STEPS.map((key, index) => (
                    <span
                        key={key}
                        className={cn(
                            'h-1 rounded-pill',
                            index < current ? 'bg-success' : index === current ? 'bg-accent' : 'bg-surface-3',
                        )}
                    />
                ))}
            </div>
        </div>
    );
}
