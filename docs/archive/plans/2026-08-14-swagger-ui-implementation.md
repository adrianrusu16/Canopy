# Bundled Swagger UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve a fully local, interactive Swagger UI for Canopy's checked-in HTTP OpenAPI document at `/docs/`.

**Architecture:** Nginx remains the owner of the public HTTP documentation surface on port 8080. A minimal pinned Swagger UI v5.32.11 distribution is checked into `deploy/swagger-ui/`, mounted read-only into both Nginx Compose variants, and configured to load the existing same-origin `/openapi.json` document.

**Tech Stack:** Nginx, Docker Compose, Swagger UI v5.32.11, OpenAPI 3.1 JSON, Rust integration tests, Bash/curl integration tests

## Global Constraints

- Preserve the existing `GET /openapi.json` endpoint and its response.
- `/docs` must redirect to the canonical `/docs/` page.
- The browser must not require a CDN or any other remote asset request.
- Swagger UI HTML, JavaScript, CSS, favicon assets, Apache-2.0 license, NOTICE, and version record must live under `deploy/swagger-ui/`.
- Keep the default Swagger UI appearance and interactive "Try it out" functionality.
- Do not add authentication, custom branding, a Rust/Axum docs route, or a separate frontend service.
- Preserve unrelated working-tree changes, including the pre-existing mode-only change to `scripts/test-streaming.sh`; stage only feature content and the intended original executable mode for feature commits.

---

## File Structure

- `deploy/swagger-ui/index.html`: small local entry page referencing only relative vendored assets.
- `deploy/swagger-ui/swagger-initializer.js`: Swagger UI bootstrap configuration pointing to `/openapi.json`.
- `deploy/swagger-ui/swagger-ui.css`: pinned upstream v5.32.11 stylesheet.
- `deploy/swagger-ui/swagger-ui-bundle.js`: pinned upstream v5.32.11 browser bundle.
- `deploy/swagger-ui/swagger-ui-standalone-preset.js`: pinned upstream v5.32.11 standalone layout preset.
- `deploy/swagger-ui/favicon-16x16.png` and `favicon-32x32.png`: pinned upstream favicon assets.
- `deploy/swagger-ui/LICENSE`, `NOTICE`, and `VERSION`: upstream licensing and exact version provenance.
- `docs/openapi.json`: documents the real `GET /docs` redirect endpoint.
- `crates/canopy-server/tests/http_openapi.rs`: validates route ownership and the checked-in UI bundle.
- `deploy/nginx/canopy-stream.conf`: serves `/docs` and `/docs/` in the container-network deployment.
- `deploy/nginx/canopy-stream.local-integration.conf`: serves the same routes in the host-network local deployment.
- `docker-compose.streaming-test.yml`: mounts the UI directory read-only for the streaming harness.
- `docker-compose.local-integration.yml`: mounts the UI directory read-only for local development.
- `scripts/test-streaming.sh`: verifies redirect, content types, assets, and absence of remote references through real Nginx.
- `README.md`: advertises the local Swagger UI URL.

---

### Task 1: Add the pinned self-contained Swagger UI bundle

**Files:**
- Create: `deploy/swagger-ui/index.html`
- Create: `deploy/swagger-ui/swagger-initializer.js`
- Create: `deploy/swagger-ui/swagger-ui.css`
- Create: `deploy/swagger-ui/swagger-ui-bundle.js`
- Create: `deploy/swagger-ui/swagger-ui-standalone-preset.js`
- Create: `deploy/swagger-ui/favicon-16x16.png`
- Create: `deploy/swagger-ui/favicon-32x32.png`
- Create: `deploy/swagger-ui/LICENSE`
- Create: `deploy/swagger-ui/NOTICE`
- Create: `deploy/swagger-ui/VERSION`
- Modify: `docs/openapi.json`
- Modify: `crates/canopy-server/tests/http_openapi.rs`

**Interfaces:**
- Consumes: the canonical OpenAPI document at repository path `docs/openapi.json` and runtime URL `/openapi.json`.
- Produces: a static UI directory whose entry point is `deploy/swagger-ui/index.html` and whose bootstrap URL is exactly `/openapi.json`.

- [ ] **Step 1: Write failing ownership and bundle tests**

Add `"/docs"` to the expected route set and add this test to `crates/canopy-server/tests/http_openapi.rs`:

```rust
use std::{fs, path::Path};

#[test]
fn swagger_ui_bundle_is_local_and_uses_the_canonical_document() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ui_root = repo_root.join("deploy/swagger-ui");
    let index = fs::read_to_string(ui_root.join("index.html"))
        .expect("Swagger UI index must be checked in");
    let initializer = fs::read_to_string(ui_root.join("swagger-initializer.js"))
        .expect("Swagger UI initializer must be checked in");

    for local_asset in [
        "./swagger-ui.css",
        "./swagger-ui-bundle.js",
        "./swagger-ui-standalone-preset.js",
        "./swagger-initializer.js",
    ] {
        assert!(index.contains(local_asset), "missing local asset {local_asset}");
    }
    assert!(!index.contains("http://"));
    assert!(!index.contains("https://"));
    assert!(initializer.contains("url: \"/openapi.json\""));
    assert!(!initializer.contains("http://"));
    assert!(!initializer.contains("https://"));

    for required_file in [
        "swagger-ui.css",
        "swagger-ui-bundle.js",
        "swagger-ui-standalone-preset.js",
        "favicon-16x16.png",
        "favicon-32x32.png",
        "LICENSE",
        "NOTICE",
        "VERSION",
    ] {
        let metadata = fs::metadata(ui_root.join(required_file))
            .unwrap_or_else(|error| panic!("missing {required_file}: {error}"));
        assert!(metadata.len() > 0, "{required_file} must not be empty");
    }

    assert_eq!(
        fs::read_to_string(ui_root.join("VERSION")).unwrap().trim(),
        "5.32.11"
    );
}
```

- [ ] **Step 2: Run the focused test to verify it fails**

Run:

```bash
cargo test -p canopy-server --test http_openapi --locked
```

Expected: FAIL because `/docs` is absent from `docs/openapi.json` and `deploy/swagger-ui/index.html` does not exist.

- [ ] **Step 3: Vendor the exact upstream distribution files**

Download the official `v5.32.11` source archive into a freshly created temporary directory, extract it, and copy only these upstream files from `dist/`: `swagger-ui.css`, `swagger-ui-bundle.js`, `swagger-ui-standalone-preset.js`, `favicon-16x16.png`, and `favicon-32x32.png`. Copy root `LICENSE` and `NOTICE`, and write `5.32.11` plus a final newline to `deploy/swagger-ui/VERSION`.

Use the pinned archive URL:

```text
https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.32.11.tar.gz
```

Do not copy source maps, development bundles, OAuth redirect support, or upstream demo specifications.

- [ ] **Step 4: Add the minimal local entry page and initializer**

Create `deploy/swagger-ui/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Canopy HTTP API</title>
    <link rel="icon" type="image/png" sizes="32x32" href="./favicon-32x32.png">
    <link rel="icon" type="image/png" sizes="16x16" href="./favicon-16x16.png">
    <link rel="stylesheet" href="./swagger-ui.css">
  </head>
  <body>
    <div id="swagger-ui"></div>
    <script src="./swagger-ui-bundle.js"></script>
    <script src="./swagger-ui-standalone-preset.js"></script>
    <script src="./swagger-initializer.js"></script>
  </body>
</html>
```

Create `deploy/swagger-ui/swagger-initializer.js`:

```javascript
window.onload = () => {
  window.ui = SwaggerUIBundle({
    url: "/openapi.json",
    dom_id: "#swagger-ui",
    deepLinking: true,
    presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
    plugins: [SwaggerUIBundle.plugins.DownloadUrl],
    layout: "StandaloneLayout",
  });
};
```

- [ ] **Step 5: Document the real redirect endpoint**

Add this path to `docs/openapi.json` without changing existing operations:

```json
"/docs": {
  "get": {
    "operationId": "getSwaggerUi",
    "summary": "Open the interactive Canopy HTTP API documentation",
    "responses": {
      "308": {
        "description": "Redirect to the canonical Swagger UI page at /docs/.",
        "headers": {
          "Location": {
            "schema": {
              "type": "string"
            }
          }
        }
      }
    }
  }
}
```

- [ ] **Step 6: Run focused verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test http_openapi --locked
git diff --check
```

Expected: formatting and the focused tests PASS; the diff has no whitespace errors.

- [ ] **Step 7: Commit the static bundle and contract**

```bash
git add deploy/swagger-ui docs/openapi.json crates/canopy-server/tests/http_openapi.rs
git commit -m "feat: vendor bundled Swagger UI"
```

### Task 2: Serve and verify Swagger UI through Nginx

**Files:**
- Modify: `deploy/nginx/canopy-stream.conf`
- Modify: `deploy/nginx/canopy-stream.local-integration.conf`
- Modify: `docker-compose.streaming-test.yml`
- Modify: `docker-compose.local-integration.yml`
- Modify: `scripts/test-streaming.sh`
- Modify: `README.md`

**Interfaces:**
- Consumes: the `deploy/swagger-ui/` directory from Task 1 and the existing Nginx port 8080 surface.
- Produces: `GET /docs` returning `308`, `GET /docs/` returning the UI HTML, and local assets below `/docs/`.

- [ ] **Step 1: Add failing real-Nginx assertions**

Immediately after the existing `/openapi.json` checks in `scripts/test-streaming.sh`, add:

```bash
docs_status="$(curl --silent --show-error --output /dev/null \
  --write-out '%{http_code}' http://127.0.0.1:18080/docs)"
[[ "$docs_status" == "308" ]]

docs_document="$(curl --fail --location --silent --show-error \
  http://127.0.0.1:18080/docs)"
grep -Fq './swagger-ui.css' <<<"$docs_document"
grep -Fq './swagger-ui-bundle.js' <<<"$docs_document"
if grep -Eq 'https?://' <<<"$docs_document"; then
  echo "Swagger UI must not load remote assets" >&2
  exit 1
fi

curl --fail --silent --show-error \
  http://127.0.0.1:18080/docs/swagger-ui.css >/dev/null
initializer="$(curl --fail --silent --show-error \
  http://127.0.0.1:18080/docs/swagger-initializer.js)"
grep -Fq 'url: "/openapi.json"' <<<"$initializer"
```

- [ ] **Step 2: Run the streaming harness to verify the new assertions fail**

Run:

```bash
bash scripts/test-streaming.sh
```

Expected: FAIL because `/docs` currently falls through to Nginx's `404` catch-all.

- [ ] **Step 3: Add Nginx documentation routes**

Before the stream route in both Nginx configuration files, add:

```nginx
location = /docs {
    return 308 /docs/;
}

location ^~ /docs/ {
    root /srv/canopy/swagger-ui-root;
    index index.html;
    try_files $uri $uri/ =404;
    add_header Cache-Control "no-cache" always;
    access_log off;
}
```

Mounting the repository directory at `/srv/canopy/swagger-ui-root/docs` makes Nginx's `root` mapping resolve `/docs/index.html` without `alias`/`try_files` path ambiguity.

- [ ] **Step 4: Mount the UI assets read-only**

Add this Nginx volume to both Compose files beside the existing OpenAPI mount:

```yaml
- ./deploy/swagger-ui:/srv/canopy/swagger-ui-root/docs:ro
```

Run:

```bash
docker compose -f docker-compose.streaming-test.yml config --quiet
docker compose -f docker-compose.local-integration.yml config --quiet
```

Expected: both Compose configurations validate successfully when their already-required environment variables are supplied by their normal harnesses.

- [ ] **Step 5: Advertise the local Swagger UI**

Update the complete local integration endpoint paragraph in `README.md` to include:

```text
Swagger UI at http://127.0.0.1:8080/docs/
```

Keep `/openapi.json` documented as the machine-readable companion.

- [ ] **Step 6: Run focused and real-boundary verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test http_openapi --locked
bash scripts/test-streaming.sh
git diff --check
```

Expected: all commands PASS. The harness confirms a `308` redirect, local HTML/CSS/JavaScript delivery, the canonical `/openapi.json` URL, and all pre-existing range/auth/revocation behavior.

- [ ] **Step 7: Commit the Nginx delivery change without absorbing unrelated mode state**

Stage only the six Task 2 paths. Ensure the staged entry for `scripts/test-streaming.sh` retains the original executable mode `100755`, while leaving any pre-existing working-tree mode-only change unstaged:

```bash
git add deploy/nginx/canopy-stream.conf \
  deploy/nginx/canopy-stream.local-integration.conf \
  docker-compose.streaming-test.yml \
  docker-compose.local-integration.yml \
  scripts/test-streaming.sh README.md
git update-index --chmod=+x scripts/test-streaming.sh
git diff --cached --check
git commit -m "feat: serve Swagger UI from nginx"
```

### Task 3: Final regression and browser verification

**Files:**
- Verify only; no planned file changes.

**Interfaces:**
- Consumes: Tasks 1 and 2 as a single local documentation surface.
- Produces: evidence that the requested UI renders and the repository remains healthy.

- [ ] **Step 1: Run the final automated checks**

```bash
cargo test -p canopy-server --test http_openapi --locked
bash scripts/test-streaming.sh
git diff --check
git status --short --branch
```

Expected: tests PASS; only unrelated pre-existing user changes remain outside the feature commits.

- [ ] **Step 2: Inspect the rendered UI**

While the local integration environment is running, open:

```text
http://127.0.0.1:8080/docs/
```

Confirm the page title is `Canopy HTTP API`, endpoint groups render from `/openapi.json`, browser developer tools show no failed or remote network requests, and "Try it out" is available.

- [ ] **Step 3: Record verification evidence**

Capture the exact successful commands and rendered-page result in the final handoff. Do not create another commit unless verification requires a corrective code change.
