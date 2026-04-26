# Phase 4 — Desktop distribution & installers (plan)

Turn the desktop app into something a non-developer can install.
Today's only install path is `cargo tauri build` locally, which
blocks anyone without a Rust toolchain from trying Orchext. Deferred
behind the web client (Phase 2b.4) and the full Phase 3 capability
expansion (3a–3e: rebrand, tasks, integrations, agent orchestration)
— the distribution story is more compelling once the product has
the task + agent surface on it.

**Renumbered from Phase 3 on 2026-04-22** when Phase 3 absorbed the
rebrand + capability expansion. Content below is unchanged in
intent; section numbers update `3.x` → `4.x`, and product references
are now **orchext** everywhere the rebrand has landed by the time
Phase 4 kicks off.

Live status in [`../implementation-status.md`](../implementation-status.md).

---

## Sub-milestones, each independently useful

### 4.1 — macOS signed + notarized DMG via CI
*([Notion](https://www.notion.so/34d47fdae49a81ac90a2e402e46dda59))*

- **Apple Developer Program enrollment** ($99/year) — manual,
  out-of-CI prerequisite. Creates the team ID used below.
- **Certificates (Apple Developer portal):**
  - "Developer ID Application" — signs the `.app` bundle
  - Export as `.p12` (password-protected)
- **App-specific password** for notarization (appleid.apple.com →
  Sign-In and Security → App-Specific Passwords).
- **GitHub Actions secrets to add:**
  - `APPLE_CERTIFICATE` — base64-encoded `.p12`
  - `APPLE_CERTIFICATE_PASSWORD` — `.p12` export password
  - `APPLE_SIGNING_IDENTITY` — e.g.
    `Developer ID Application: JB Butler (TEAMID)`
  - `APPLE_ID` — developer account email
  - `APPLE_PASSWORD` — app-specific password
  - `APPLE_TEAM_ID` — 10-char team identifier
- **`.github/workflows/release.yml`** — triggered on `v*` tag push:
  1. `macos-14` runner.
  2. Decode `APPLE_CERTIFICATE`, `security import` into a temp
     keychain, unlock it.
  3. `npm ci` in `apps/desktop/` + workspace build.
  4. `cargo tauri build` — Tauri picks up `APPLE_SIGNING_IDENTITY`
     from env and signs the `.app`.
  5. `xcrun notarytool submit <dmg> --apple-id $APPLE_ID
     --password $APPLE_PASSWORD --team-id $APPLE_TEAM_ID --wait`
  6. `xcrun stapler staple <dmg>`
  7. `gh release create vX.Y.Z <dmg> --generate-notes`
- **`tauri.conf.json` additions** under `bundle.macOS`:
  - `signingIdentity` → read from env (Tauri supports `$` expansion)
  - `providerShortName` → team ID
  - `entitlements` → hardened runtime entitlements plist
    (no camera/mic; filesystem read/write for user-chosen dirs)
- **First tagged release** — `v0.1.0` once Phase 3 stabilizes.
- **Cuts:** no auto-updater yet; users re-download from Releases.
  No universal binary yet — ship Apple Silicon only; add Intel
  target if anyone asks.

### 4.2 — Windows signed MSI
*([Notion](https://www.notion.so/34d47fdae49a8156870fcccef4cbb2ae))*

- **Code signing cert** — SSL.com / DigiCert / Certum (~$100–400/yr
  for EV or OV) or **Azure Trusted Signing** (pay-per-use, no
  hardware token). Prefer Azure Trusted Signing for CI ergonomics.
- Add `windows-latest` job to `release.yml` that runs
  `cargo tauri build --target x86_64-pc-windows-msvc` and signs
  the produced `.msi` via `signtool` + the Azure signing action.
- **Secrets:** `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
  `AZURE_CLIENT_SECRET`, `AZURE_TRUSTED_SIGNING_ACCOUNT`,
  `AZURE_TRUSTED_SIGNING_PROFILE`.
- **Cut:** no SmartScreen reputation for a while — first users
  will see "unrecognized app" warnings until install volume builds.

### 4.3 — Linux AppImage + `.deb`
*([Notion](https://www.notion.so/34d47fdae49a811393f1eaa509d25d45))*

- `ubuntu-22.04` runner job that produces both artifacts via
  `cargo tauri build`.
- No signing story (distros handle trust differently); SHA-256
  sums published alongside the release.
- **Cuts:** no Flatpak/Snap; no APT/YUM repo hosting — direct
  download from Releases only.

### 4.4 — Auto-updater
*([Notion](https://www.notion.so/34d47fdae49a8171b5e6e1dec05fee4b))*

- Tauri `updater` plugin pointed at a signed JSON manifest.
- **Manifest hosting:** GitHub Releases raw asset URL works for
  v1; revisit a CDN-backed endpoint if traffic justifies it.
- **Signing:** generate a `minisign` / `age` keypair; private key
  as a GitHub Actions secret, public key baked into the app at
  build time. Manifest signed per release by the workflow.
- Per-platform update artifacts produced by 4.1–4.3 and referenced
  in the manifest.
- **Defer until:** at least Phase 3a (rebrand + tasks) has shipped
  and stabilized. Shipping a broken auto-update is worse than no
  auto-update — users can't roll back themselves.

### 4.5 — Download landing page (optional)
*([Notion](https://www.notion.so/34d47fdae49a81f89104fe82a3890b46))*

- Static page (e.g., `orchext.app/download`) with OS-detection and
  a "Download for macOS/Windows/Linux" button pointing at the
  latest GitHub Release asset.
- **Cut unless needed:** GitHub Releases is a fine download URL
  until we care about branding.

## Unblocks

- Handing the app to non-developer testers.
- First real install telemetry (crash reports if we add them later).
- A website download button that isn't `git clone && cargo build`.

## Open questions

- **Beta vs. stable channels** — probably not until we have >1
  user who cares. Single channel to start.
- **Crash reporting / telemetry** — off by default, opt-in only
  (matches the self-host-first positioning). Sentry vs. a custom
  endpoint is a 4.4+ question.
- **Homebrew cask** — would let `brew install --cask orchext` work.
  Easy to publish once 4.1 produces a signed DMG; add when there's
  demand.
