// The source catalog. Every other locale is typed against it, so a key missing there fails the type
// check. Messages that take values are functions; they format with the helpers bound to this locale.

export const en = {
    app: {
        name: 'Observer',
        skipToContent: 'Skip to content',
    },
    nav: {
        label: 'Main navigation',
        open: 'Open the menu',
        close: 'Close the menu',
        overview: 'Overview',
    },
    theme: {
        label: 'Theme',
        dark: 'Dark',
        light: 'Light',
    },
    status: {
        observer: 'Observer',
        version: 'Version',
        ok: 'Running',
        down: 'Unreachable',
        pending: 'Checking',
    },
    overview: {
        title: 'Overview',
        gloss: 'Observer, right now.',
        stateTitle: 'State',
        running: 'Observer is running.',
        version: (version: string) => `Version ${version}`,
        unreachable: 'Observer cannot be reached.',
        unreachableHint: 'The panel keeps trying every few seconds.',
        loading: 'Checking Observer',
    },
    errors: {
        notFound: 'This page does not exist.',
        backToOverview: 'Back to the overview',
    },
};

export type Messages = typeof en;
