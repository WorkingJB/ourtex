# Deploy

Configuration for the hosted SaaS deployment of Orchext. Self-hosters
do **not** need anything in this directory — they use
`crates/orchext-server/Dockerfile` + `docker-compose.yml` directly.

The full architecture, rationale, and decision log live in
[`docs/DEPLOYMENT.md`](../docs/DEPLOYMENT.md). This README is the
operational quick-reference.

## Layout

```
deploy/
├── fly/
│   ├── orchext-prod.toml   # Fly app config — production
│   └── orchext-test.toml   # Fly app config — test/staging
└── vercel/
    └── README.md            # Vercel project bring-up notes (no
                             # in-repo config — Vercel is UI-driven)
```

## First-time bring-up (high level)

In order:

1. **Domains and DNS** — register `orchext.ai` (or whatever) at a
   registrar that supports CNAME-at-apex (Cloudflare DNS, Route 53).
   Don't entangle DNS with Vercel or Fly.

2. **Neon Postgres** — create two projects: `orchext-prod` and
   `orchext-test`, both in AWS `us-west-2`. Grab the *direct*
   (unpooled) connection string for each — sqlx 0.8 has its own pool
   and doesn't need pgbouncer. See
   [`docs/DEPLOYMENT.md` §5.3](../docs/DEPLOYMENT.md#53-connection-string-discipline).

3. **Fly apps** — create both apps and inject the Neon URLs as
   secrets, then deploy. **Run from the repo root** so `.` is the
   build context (the Dockerfile expects to see the entire workspace):
   ```bash
   flyctl apps create orchext-prod
   flyctl secrets set DATABASE_URL='postgres://...' --app orchext-prod
   flyctl deploy . --config deploy/fly/orchext-prod.toml

   flyctl apps create orchext-test
   flyctl secrets set DATABASE_URL='postgres://...' --app orchext-test
   flyctl deploy . --config deploy/fly/orchext-test.toml
   ```

4. **Vercel projects** — follow [`vercel/README.md`](vercel/README.md).
   The two projects rewrite `/v1/*` to their respective Fly app.

5. **DNS records** — add CNAMEs at the registrar:
   - `app.orchext.ai` → `cname.vercel-dns.com`
   - `test-app.orchext.ai` → `cname.vercel-dns.com`

6. **Smoke test**:
   ```bash
   curl https://app.orchext.ai/healthz       # → {"ok":true} (via Vercel rewrite)
   curl https://orchext-prod.fly.dev/readyz  # → {"ok":true}
   ```

## Day-to-day deploys

**Convention: test first, prod by promotion.** Work lands on
`develop`, ships to `test-app.orchext.ai` automatically, gets
verified, then is promoted to `main` (and to prod) by a fast-forward
merge. Don't commit directly to `main`.

### Web app

1. Land work on `develop` (PR or direct commit).
2. Vercel auto-deploys to `test-app.orchext.ai`. Smoke-test it.
3. To ship to prod, fast-forward `main` from `develop` and push:
   ```bash
   git checkout main
   git pull --ff-only
   git merge --ff-only develop
   git push origin main
   ```
   Vercel auto-deploys `app.orchext.ai`. The FF guarantees prod
   never has commits that didn't go through test.

### Server (Fly)

Automated. `.github/workflows/fly-deploy.yml` deploys the server to
Fly on push:

| Branch pushed | Fly app deployed |
|---|---|
| `develop` | `orchext-test` |
| `main` | `orchext-prod` |

The workflow is gated by a paths filter — only changes to
`crates/**`, `Cargo.{toml,lock}`, or `deploy/fly/**` trigger a build,
so doc-only or web-only commits don't burn a Rust build.

Requires two repo secrets, both scoped deploy tokens:

| Secret | How to mint | Used for |
|---|---|---|
| `FLY_API_TOKEN_TEST` | `flyctl tokens create deploy --app orchext-test --name "github-actions-test" --expiry 8760h` | `develop` → `orchext-test` |
| `FLY_API_TOKEN_PROD` | `flyctl tokens create deploy --app orchext-prod --name "github-actions-prod" --expiry 8760h` | `main` → `orchext-prod` |

Scoped deploy tokens (vs. `flyctl auth token`) limit the blast radius
of a leak to the single Fly app — they can't read other secrets,
deploy other apps, or destroy machines. Rotate by re-running the
`flyctl tokens create` command and pasting the new value over the
secret in GitHub.

For ad-hoc rollouts (debugging, hotfix from a dev machine), manual
deploy still works:

```bash
flyctl deploy . --config deploy/fly/orchext-test.toml   # test first
flyctl deploy . --config deploy/fly/orchext-prod.toml   # then prod
```

Run from the repo root so the build context covers the whole
workspace.

## Costs

See [`docs/DEPLOYMENT.md` §9](../docs/DEPLOYMENT.md#9-cost-expectations).
Expect ~$35/mo combined to start; ~$100/mo when HA Postgres is
warranted.

## Migration paths

If a vendor needs to be replaced, see
[`docs/DEPLOYMENT.md` §12](../docs/DEPLOYMENT.md#12-migration-paths).
Every component is plain Postgres / plain Docker / plain DNS — no
vendor lock-in beyond config.
