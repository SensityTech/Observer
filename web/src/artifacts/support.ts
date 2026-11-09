import type { SupportLevel } from '../api/generated/SupportLevel';

/** The tone of a build's support chip: green while supported, then warning, then danger. */
export const supportTone = (support: SupportLevel) =>
    support === 'endOfLife'
        ? 'danger'
        : support === 'endOfSupport'
          ? 'warning'
          : support === 'supported'
            ? 'success'
            : 'neutral';
