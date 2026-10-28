// The second half of a session, next to the HttpOnly cookie. Cookies are shared by every port of a
// host and localStorage is not, so a cookie read by a page FXServer serves on :30120 is useless alone.
const STORAGE_KEY = 'observer.sessionKey';

export const readSessionKey = () => {
    try {
        return localStorage.getItem(STORAGE_KEY);
    } catch {
        return null;
    }
};

export const saveSessionKey = (key: string) => {
    try {
        localStorage.setItem(STORAGE_KEY, key);
    } catch {
        // Storage blocked: every request after this one is refused, and the panel shows the login.
    }
};

export const clearSessionKey = () => {
    try {
        localStorage.removeItem(STORAGE_KEY);
    } catch {
        // Nothing was stored.
    }
};
