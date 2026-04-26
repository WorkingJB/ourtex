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

| What | How |
|---|---|
| Web app (prod) | Push to `main` — Vercel auto-deploys |
| Web app (test) | Push to `develop` — Vercel auto-deploys |
| Server (prod) | `flyctl deploy . --config deploy/fly/orchext-prod.toml` (run from repo root) |
| Server (test) | `flyctl deploy . --config deploy/fly/orchext-test.toml` (run from repo root) |

CI workflows in `.github/workflows/` cover build + test gates. The
deploy step is manual/CLI for now — automating Fly deploys via tag
push is deferred until we have enough release cadence to justify it.

## Costs

See [`docs/DEPLOYMENT.md` §9](../docs/DEPLOYMENT.md#9-cost-expectations).
Expect ~$35/mo combined to start; ~$100/mo when HA Postgres is
warranted.

## Migration paths

If a vendor needs to be replaced, see
[`docs/DEPLOYMENT.md` §12](../docs/DEPLOYMENT.md#12-migration-paths).
Every component is plain Postgres / plain Docker / plain DNS — no
vendor lock-in beyond config.
