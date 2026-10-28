import { useEffect, useId, useLayoutEffect, useRef, type FormEvent, type ReactNode } from 'react';

import { cn } from '../lib/cn';

type DialogProps = {
    title: ReactNode;
    onClose: () => void;
    children: ReactNode;
    footer: ReactNode;
    /** Makes the dialog a form: Enter in a field submits it. */
    onSubmit?: () => void;
    /** A click on the backdrop closes the dialog, except while it holds something typed. */
    closeOnBackdrop?: boolean;
    role?: 'dialog' | 'alertdialog';
    className?: string;
};

/**
 * A modal dialog on the native element: the browser traps the focus, makes the page inert and
 * closes it on Escape. Mount it to open it; it closes when its owner unmounts it.
 */
export function Dialog({
    title,
    onClose,
    children,
    footer,
    onSubmit,
    closeOnBackdrop = true,
    role,
    className,
}: DialogProps) {
    const ref = useRef<HTMLDialogElement>(null);
    const titleId = useId();

    // A layout effect's cleanup runs while the element is still in the page, so close() can hand the
    // focus back to the trigger; removed open, the dialog would drop it on the body.
    useLayoutEffect(() => {
        const dialog = ref.current;
        dialog?.showModal();
        // Chrome counts a body that scrolls as the first thing to focus: start on the first field.
        dialog?.querySelector<HTMLElement>('input:not([type="hidden"]):enabled, select:enabled')?.focus();
        return () => dialog?.close();
    }, []);

    useEffect(() => {
        const dialog = ref.current;
        if (dialog === null || !closeOnBackdrop) {
            return;
        }
        // The dialog box fills its element, so a press on the element itself hit the backdrop. Both
        // ends of the click count: a text selection dragged out of a field must not close it.
        let pressedOutside = false;
        const press = (event: PointerEvent) => {
            pressedOutside = event.target === dialog;
        };
        const release = (event: MouseEvent) => {
            if (pressedOutside && event.target === dialog) {
                onClose();
            }
        };
        dialog.addEventListener('pointerdown', press);
        dialog.addEventListener('click', release);
        return () => {
            dialog.removeEventListener('pointerdown', press);
            dialog.removeEventListener('click', release);
        };
    }, [closeOnBackdrop, onClose]);

    const submit = (event: FormEvent) => {
        event.preventDefault();
        onSubmit?.();
    };

    const content = (
        <>
            <div className="flex items-start gap-3.5 px-5 pt-5">
                <h2 id={titleId} className="text-[17px] leading-snug font-semibold tracking-snug text-ink">
                    {title}
                </h2>
            </div>
            <div className="flex min-h-0 flex-col gap-4 overflow-y-auto px-5 pt-2.5 pb-5 text-body text-ink-2">
                {children}
            </div>
            <div className="flex flex-wrap justify-end gap-2 rounded-b-dialog border-t border-line bg-bg-subtle px-5 py-3.5 max-md:[&>button]:flex-1">
                {footer}
            </div>
        </>
    );

    return (
        <dialog
            ref={ref}
            role={role}
            aria-labelledby={titleId}
            className={cn(
                'mx-auto mt-24 mb-auto max-h-[calc(100dvh-8rem)] w-[calc(100%-2rem)] max-w-dialog flex-col overflow-hidden rounded-dialog border border-line bg-surface p-0 text-ink shadow-lg open:flex open:animate-rise max-md:mt-16',
                'backdrop:bg-overlay backdrop:animate-fade',
                className,
            )}
            onCancel={(event) => {
                event.preventDefault();
                onClose();
            }}
        >
            {onSubmit ? (
                <form className="flex min-h-0 flex-col" onSubmit={submit} noValidate>
                    {content}
                </form>
            ) : (
                content
            )}
        </dialog>
    );
}
