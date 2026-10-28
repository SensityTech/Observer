// Error parameters arrive as untyped JSON: a message reads them through these, never trusting a shape.

export const numberParam = (params: Record<string, unknown>, name: string) => {
    const value = params[name];
    return typeof value === 'number' && Number.isFinite(value) ? value : 0;
};

export const stringParam = (params: Record<string, unknown>, name: string) => {
    const value = params[name];
    return typeof value === 'string' ? value : '';
};
