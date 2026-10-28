// Applies the saved theme before the first paint. It lives outside the bundle so the content
// security policy can stay `script-src 'self'` with no inline script.
try {
    var theme = localStorage.getItem('observer.theme');
    if (theme === 'light' || theme === 'dark') {
        document.documentElement.dataset.theme = theme;
    }
} catch (error) {
    // Storage unavailable: keep the default theme from index.html.
}
