# RustySoul as an installed web app

Open your **daemon's web interface**, then use your browser's Install app option. Settings -> Appearance also offers **Install RustySoul** when the browser supplies an installation prompt. On iPhone/iPad, use Safari -> Share -> Add to Home Screen. Installation support and labels vary by browser.

Use HTTPS for a remote server, or HTTP on localhost/127.0.0.1 for local use or an SSH tunnel. Plain HTTP on a LAN IP does not meet manifest-based installation requirements. See [connection settings](CONNECTIONS.md) for binding/reverse-proxy configuration and [browser requirements](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Guides/Making_PWAs_installable).

The installed app has a standalone window, the supplied project icon and a stable app identity at the daemon's origin. Normal authentication and session expiry still apply. The server must remain reachable; the daemon continues transfers when the browser/app is closed. No service worker or private offline cache is introduced. A service worker is not required for installation in supporting browsers.

The manifest is `/site.webmanifest`. One self-contained garden-themed SVG, `images/rustysoul.svg`, supplies the browser/favicon, touch-icon link, installed-app icon, sidebar branding and login branding. It includes the rounded dark-green box (`#344332`) and pale-green waveform (`#d7e7ae`), so it needs no CSS or external resources. Vite serves it at `/assets/icons/rustysoul.svg` during development and embeds the same file in Docker/native builds. The manifest declares `image/svg+xml`, `sizes: any` and `purpose: any`. The original PNG artwork in `images/` is not bundled or referenced by the interface.

SVG-only icons require a browser that supports SVG icons in the relevant context. Safari 26 introduced SVG icons across its browser interface and web-app Home Screen/Dock surfaces; see [WebKit's release notes](https://webkit.org/blog/17333/webkit-features-in-safari-26-0/#svg-icons). Older browsers may use a generic or generated icon. No PNG fallbacks are shipped, as requested.

The GitHub project website uses the matching browser/Apple favicons. It does not advertise itself as the daemon app; install from your server's URL instead.
