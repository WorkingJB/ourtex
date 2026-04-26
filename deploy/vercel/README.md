# Vercel deployment

Vercel hosts the static `apps/web` build — the Vite/React SPA. Two
projects, one per environment:

| Project | Domain | Deploys from |
|---|---|---|
| `orchext-web-prod` | `app.orchext.ai` | `main` |
| `orchext-web-test` | `test-app.orchext.ai` | `develop` (or `main` until a `develop` branch exists) |

Vercel itself is configured through its UI/API rather than a
committed config file. There is no `fly.toml`-style equivalent worth
maintaining here. This README captures the *expected* state so it
can be re-created or audited.

## First-time bring-up

For each project (`-prod` and `-test`):

1. **Create the project** in Vercel, point it at this repo.
2. **Build settings**:
   - Framework preset: **Vite**
   - Root directory: `apps/web`
   - Build command: `npm run build`
   - Output directory: `dist`
   - Install command: `npm ci`
3. **Production branch**: `main` for prod project; `develop` for test
   project.
4. **Domains**: bind `app.orchext.ai` (or `test-app.orchext.ai`) and
   add the CNAME record at the registrar pointing to
   `cname.vercel-dns.com`.
5. **Environment variables** (per project, all environments):
   - `VITE_ENV_NAME` = `production` or `test` (cosmetic — drives any
     env-banner UI).
6. **Rewrites**: configure under *Project Settings → Rewrites*.
   Each project gets its own rewrites (prod and test point at
   different Fly apps), so we don't commit a `vercel.json` — that
   would lock both projects into the same destination.

   For each project, add two rewrites mapping the API and healthcheck
   paths to the matching Fly hostname:

   | Source | Production destination | Test destination |
   |---|---|---|
   | `/v1/:path*` | `https://orchext-prod.fly.dev/v1/:path*` | `https://orchext-test.fly.dev/v1/:path*` |
   | `/healthz`   | `https://orchext-prod.fly.dev/healthz`   | `https://orchext-test.fly.dev/healthz` |

   Why no `vercel.json`: the file would need different destinations
   per environment, and Vercel rewrites don't support env-var
   substitution. UI-only avoids a stale-URL footgun.

## Why no Vercel Functions

We deliberately do **not** use Vercel Functions (Edge or Node) to
proxy the API. Reasons:

- The Rust API + Postgres needs a long-running process — Vercel
  Functions don't fit that shape.
- Rewrites are zero-runtime: Vercel just forwards the request without
  running our code. Lower latency, no cold start, no per-invocation
  billing.
- Splitting the API origin onto Fly keeps the deploy story
  symmetric with self-hosters (same `Dockerfile`, just running
  somewhere else).

## What lives in this repo vs. in Vercel

| Lives in repo | Lives in Vercel |
|---|---|
| `apps/web/` source | Project itself, build/install config |
| `apps/web/vercel.json` (rewrites) | Domain bindings, DNS verification |
| `VITE_*` env names referenced by code | Env values per environment |
| Build command (Vercel reads from project settings) | Production-branch mapping |

Treat the Vercel UI as a deployment target, not a source of truth.
If a setting is meaningful enough to track, it belongs in code or in
[`docs/DEPLOYMENT.md`](../../docs/DEPLOYMENT.md).
