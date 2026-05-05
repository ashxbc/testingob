# Liquidity Vacuum

Real-time BTC orderbook intelligence. Watches the Binance L2 book, identifies large limit-order walls at meaningful price levels, and detects when those walls vanish — a "liquidity vacuum" — disambiguating cancellations (signal) from fills (noise).

A 15-minute directional model fuses orderbook imbalance, signed trade flow, vacuum bias and remaining wall pressure into a price target with a confidence score.

## Architecture

```
Binance WS ──► ingestor (Rust)  ──► Redis pub/sub ──► analyzer (Rust) ──► Redis state
                                                                  │
                                                                  ▼
                                                            api (Rust, axum + SSE)
                                                                  │
                                                                  ▼
                                                            web (Next.js)
```

Four services on one VPS, each restart-on-failure:

- **`ingestor/`** — `BTCUSDT@depth@100ms` + `aggTrade` streams; maintains a local L2 book, identifies walls (≥ $1.5M *and* ≥ 5× neighborhood median), classifies disappearances as `cancelled` / `filled` / `mixed` using the trade tape.
- **`analyzer/`** — rolling 5/15-min features (OFI, CVD, vacuum imbalance, wall pressure, ATR), runs the prediction model every 1s.
- **`api/`** — REST + Server-Sent Events on `:8080`.
- **`web/`** — Next.js, mobile-first, system-aware light/dark, built around a single price card + 15-min outlook + walls + vacuum feed.

## The 15-minute model

```
direction_score = 0.30·OFI_5m + 0.25·CVD_slope + 0.30·vacuum_bias + 0.15·wall_pressure
magnitude_bps   = ATR_15m_bps · |direction_score| · (0.5 + thinness)
target_price    = mid · (1 + sign(direction_score) · magnitude_bps / 10_000)
confidence      = 0.6·signal_agreement + 0.4·|direction_score|
```

Vacuums classified as `cancelled` get full weight; `mixed` half; `filled` near-zero — only deliberate wall removal counts as institutional intent.

## Local development

Requires Rust 1.74+, Node 20+, Redis 7+.

```bash
# 1. Redis
docker run -d --name lv-redis -p 6379:6379 redis:7-alpine

# 2. Backend (3 terminals or use `cargo run -p <svc> &`)
cargo run -p ingestor --release
cargo run -p analyzer --release
cargo run -p api --release

# 3. Frontend
cd web
npm install
npm run dev    # http://localhost:3000
```

Configuration is loaded from `config.toml` then overridden by env vars (`LV__SYMBOL=BTCUSDT`, `LV__REDIS_URL=...`).

## VPS deployment

### Option A — Docker Compose (recommended)

```bash
cd deploy
docker compose up -d --build
# Web on 127.0.0.1:3000, API on 127.0.0.1:8080
# Front with nginx + Let's Encrypt (deploy/nginx.conf as template)
```

### Option B — systemd

1. Build release binaries: `cargo build --release` → copy to `/opt/liquidity/bin/`.
2. Build the web app: `cd web && npm ci && npm run build` → copy `.next/standalone/`, `.next/static/`, `public/` to `/opt/liquidity/web/`.
3. Install Redis: `apt install redis-server` and enable persistence in `/etc/redis/redis.conf` (`appendonly yes`).
4. Copy the four unit files from `deploy/systemd/` to `/etc/systemd/system/` and:
   ```bash
   useradd -r -s /usr/sbin/nologin lv
   chown -R lv:lv /opt/liquidity
   systemctl daemon-reload
   systemctl enable --now lv-ingestor lv-analyzer lv-api lv-web
   ```
5. Reverse proxy: drop `deploy/nginx.conf` into `/etc/nginx/sites-available/`, edit the `server_name`, then `certbot --nginx -d your.domain.tld`.

All four services use `Restart=always` with watchdog timers. Both the depth and trade WS clients auto-reconnect with exponential backoff and re-fetch the REST snapshot on every reconnect to stay sequenced.

## Reliability notes

- **Snapshot/delta sequencing** — REST snapshot is fetched after the WS connects; deltas are buffered and replayed once `lastUpdateId` is established. A gap forces full resync.
- **Wall identity** — walls are keyed by exact price; size changes within tolerance are tracked, so partial fills don't fire false vacuums.
- **Cancel vs fill** — the trade tape is bucketed by integer dollar; on disappearance, `traded_qty / wall_qty` classifies the pull. Threshold-tuneable in `walls.rs`.
- **Backpressure** — book updates are throttled to 10 Hz on the publish side; raw WS is processed every 100 ms by Binance.

## Files of interest

| File | Purpose |
|---|---|
| [common/src/events.rs](common/src/events.rs) | Wire types shared across services |
| [ingestor/src/orderbook.rs](ingestor/src/orderbook.rs) | L2 book mirror |
| [ingestor/src/walls.rs](ingestor/src/walls.rs) | Wall + vacuum detection |
| [ingestor/src/stream.rs](ingestor/src/stream.rs) | Binance WS clients |
| [analyzer/src/features.rs](analyzer/src/features.rs) | 5/15-min rolling features |
| [analyzer/src/model.rs](analyzer/src/model.rs) | 15-min predictive model |
| [api/src/main.rs](api/src/main.rs) | REST + SSE |
| [web/app/page.tsx](web/app/page.tsx) | Dashboard |

## License

MIT.
