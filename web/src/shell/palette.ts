import { createContext, use } from 'react';

/** Opens the panel's search, for a page that offers it besides the rail's button and Ctrl K. */
export const PaletteContext = createContext<() => void>(() => {});

export const useOpenPalette = () => use(PaletteContext);
