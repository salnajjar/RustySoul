# Project website

Live at **https://salnajjar.github.io/RustySoul/**, hosted by GitHub Pages.

The site is plain HTML, CSS and JavaScript. It uses the application's garden palette and the three project screenshots in `screens/`. It needs no frontend build tool, external font, analytics service or runtime API.

From the repository root, build a local preview:

```sh
python3 scripts/build-site.py
python3 -m http.server 8766 --bind 127.0.0.1 --directory artifacts/site
```

Open http://127.0.0.1:8766. The build copies only the four selected site files, three screenshots and the garden-themed `images/rustysoul.svg` icon; private docs and other artifacts are not publication inputs. The screenshots and SVG each have one canonical source; the same self-contained icon is emitted as `assets/mark.svg` for the site branding, favicon and touch-icon link. The installable web-app manifest belongs to the daemon UI, not this project landing page; see [web app support](../docs/WEB_APP.md).

Changes to `site/`, `screens/`, `images/`, the builder or the Pages workflow deploy automatically on `main`. The Pages publishing source is **GitHub Actions**. `pages.yml` uploads only the isolated site output, with separate permissions from application/container releases. Website-only changes skip the application CI workflow; mixed changes and beta releases still run it.

When releasing another beta, update the version links, native download URLs and image references in `index.html`. Keep prerequisites and commands consistent with the main README and installation guides. Review desktop/mobile layout, screenshot dialogs, keyboard tab navigation and command copying after interactive changes.
