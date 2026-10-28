import { Component, type ReactNode } from 'react';

type Props = { fallback: ReactNode; children: ReactNode };

/** Keeps a failure of its children to them: the rest of the page stays. */
export class ErrorBoundary extends Component<Props, { failed: boolean }> {
    override state = { failed: false };

    static getDerivedStateFromError() {
        return { failed: true };
    }

    override render() {
        return this.state.failed ? this.props.fallback : this.props.children;
    }
}
