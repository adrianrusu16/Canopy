# Bundled Swagger UI design

## Goal

Expose an interactive Swagger UI for Canopy's existing HTTP OpenAPI document without adding a separate frontend service or requiring internet access.

## Scope

- Preserve the existing `GET /openapi.json` endpoint and its response.
- Add `/docs`, redirecting to the canonical documentation page at `/docs/`.
- Bundle Swagger UI HTML, JavaScript, CSS, and other assets into the local Nginx deployment.
- Configure the UI to load Canopy's existing OpenAPI document.
- Keep the default Swagger UI appearance and interactive "Try it out" functionality.

Authentication, custom branding, API documentation restructuring, and a separately deployed frontend are outside this change.

## Architecture

Nginx owns the public HTTP documentation surface on port 8080 and already serves `docs/openapi.json` directly. Swagger UI therefore belongs in Nginx rather than the Rust/Axum process.

Vendor the official Swagger UI v5.32.11 distribution under `deploy/swagger-ui/`. Configure both Nginx variants to redirect `/docs` to `/docs/` and serve the vendored directory below `/docs/`. Mount that directory read-only in the streaming-test and local-integration Nginx containers. The Swagger UI bootstrap configuration will reference the same-origin `/openapi.json` URL so there remains one canonical OpenAPI document.

All UI assets must be checked into the repository and served locally. The browser must not fetch scripts, stylesheets, fonts, or other required resources from a public CDN. Retain the upstream license notice alongside the vendored distribution and record the pinned Swagger UI version in the vendored directory.

## Request flow

1. A developer opens `/docs`.
2. Nginx redirects the request to `/docs/` and returns the vendored Swagger UI page and assets.
3. Swagger UI requests `/openapi.json` from the same origin.
4. The UI renders the operations and sends "Try it out" requests to the same local server.

## Error behavior

- If the OpenAPI document cannot be loaded, Swagger UI displays its normal fetch error rather than hiding it behind custom JavaScript.
- Existing API and OpenAPI routing behavior must remain unchanged.
- No network dependency may be required for the docs page to render.
- Unknown paths below `/docs/` must return `404` rather than falling through to another Canopy route.

## Verification

- Run the repository's formatter and relevant test suite.
- Verify `/openapi.json` still returns a successful JSON response.
- Verify `/docs` redirects to `/docs/` and `/docs/` returns HTML successfully.
- Verify the documentation HTML and bundled assets are served locally.
- Verify the documentation HTML contains no CDN or other remote asset references.
- Add `/docs` to the checked-in OpenAPI document and its exact-route ownership test.
- Extend the existing streaming integration harness to assert the route, content type, bundled JavaScript, and bundled CSS.
