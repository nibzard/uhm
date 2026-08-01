# uhm telemetry gateway

This independently deployable Cloudflare Worker accepts only the exact versioned, enum-only payloads produced by the released v1 client and the current v2 client. It rejects bodies at or above 2 KiB, unknown fields or enum variants, unsupported versions, non-JSON requests, and requests over the coarse constant-key rate limit. It writes one Workers Analytics Engine point and returns 202.

The v0.1 deployment is `https://uhm-telemetry.nikola-balic.workers.dev`. The preferred `telemetry.uhm.dev` hostname is deferred because that domain is not currently owned or delegated; the CLI and privacy documentation name the actual Cloudflare hostname instead.

Worker observability and invocation logs are disabled in `wrangler.jsonc`. The code never reads or records request headers, `request.cf`, IP address, geolocation, User-Agent, URL query data, or an exact client timestamp. Cloudflare still processes connection metadata to serve the HTTPS request, and Analytics Engine adds its server ingestion time.

## Operations

```sh
npm install
npm test
npm run deploy
```

Set `ENABLED` to `false` as the server-side kill switch and redeploy. The Analytics Engine binding creates `uhm_cli_v1` on first write. Raw data retention is three months. The current Free allowances checked on 2026-08-01 are 100,000 data points and 10,000 read queries per day; Cloudflare says Analytics Engine usage is currently unbilled while publishing future pricing.

The field order is documented in [queries.sql](queries.sql). Aggregate counts use `SUM(_sample_interval)` because Analytics Engine may sample. The release validation sent one synthetic, content-free interaction and successfully ran all three queries against the live dataset. Never enable payload logging or copy raw events to another store.

Operational limits and prices are time-sensitive. Recheck the official [Analytics Engine pricing](https://developers.cloudflare.com/analytics/analytics-engine/pricing/), [Analytics Engine limits](https://developers.cloudflare.com/analytics/analytics-engine/limits/), and [Workers limits](https://developers.cloudflare.com/workers/platform/limits/) before changing the deployment.
