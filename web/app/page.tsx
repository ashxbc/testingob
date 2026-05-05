'use client';

import { useEffect, useState } from 'react';
import styles from './page.module.css';
import { Prediction, BookSnapshot, Wall, VacuumEvent } from '@/lib/types';
import { fmtUSD, fmtCompact, fmtBps, fmtTimeAgo } from '@/lib/format';

export default function Home() {
  const [book, setBook] = useState<BookSnapshot | null>(null);
  const [predict, setPredict] = useState<Prediction | null>(null);
  const [walls, setWalls] = useState<Wall[]>([]);
  const [vacuums, setVacuums] = useState<VacuumEvent[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [b, p, w, v] = await Promise.all([
        fetch('/api/state').then((r) => r.json()).catch(() => null),
        fetch('/api/predict').then((r) => r.json()).catch(() => null),
        fetch('/api/walls').then((r) => r.json()).catch(() => []),
        fetch('/api/vacuums').then((r) => r.json()).catch(() => []),
      ]);
      if (cancelled) return;
      if (b) setBook(b);
      if (p) setPredict(p);
      if (Array.isArray(w)) setWalls(w);
      if (Array.isArray(v)) setVacuums(v);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const es = new EventSource('/api/stream');
    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.addEventListener('book', (e) =>
      setBook(JSON.parse((e as MessageEvent).data))
    );
    es.addEventListener('predict', (e) =>
      setPredict(JSON.parse((e as MessageEvent).data))
    );
    es.addEventListener('walls', (e) =>
      setWalls(JSON.parse((e as MessageEvent).data))
    );
    es.addEventListener('vacuum', (e) => {
      const v = JSON.parse((e as MessageEvent).data);
      setVacuums((prev) => [v, ...prev].slice(0, 50));
    });
    return () => es.close();
  }, []);

  const sortedWalls = [...walls].sort(
    (a, b) => Math.abs(a.distance_bps) - Math.abs(b.distance_bps)
  );
  const bidWalls = sortedWalls.filter((w) => w.side === 'bid').slice(0, 5);
  const askWalls = sortedWalls.filter((w) => w.side === 'ask').slice(0, 5);

  return (
    <main className={styles.main}>
      <header className={styles.header}>
        <div className={styles.brand}>
          <span className={styles.logo}>◐</span>
          <span>Liquidity Vacuum</span>
        </div>
        <div className={styles.symbol}>
          <span>BTC</span>
          <span className={`${styles.dot} ${connected ? styles.live : ''}`} />
        </div>
      </header>

      <section className={styles.priceCard}>
        <div className={styles.priceLabel}>Mid price</div>
        <div className={`${styles.price} tnum`}>
          {book ? fmtUSD(book.mid) : '—'}
        </div>
        <div className={styles.priceMeta}>
          <span className="tnum">Bid {book ? fmtUSD(book.best_bid) : '—'}</span>
          <span className={styles.sep}>·</span>
          <span className="tnum">Ask {book ? fmtUSD(book.best_ask) : '—'}</span>
          <span className={styles.sep}>·</span>
          <span className="tnum">
            Spread {book ? fmtBps(book.spread_bps) : '—'}
          </span>
        </div>
      </section>

      <section className={styles.predictCard}>
        <div className={styles.cardHead}>
          <div className={styles.cardTitle}>15-minute outlook</div>
          {predict && (
            <div
              className={`${styles.confBadge} ${
                predict.direction > 0
                  ? styles.up
                  : predict.direction < 0
                  ? styles.down
                  : styles.neutral
              }`}
            >
              {predict.label}
            </div>
          )}
        </div>
        {predict ? (
          <>
            <div className={styles.targetRow}>
              <div>
                <div className={styles.subLabel}>Target</div>
                <div
                  className={`${styles.targetPrice} tnum ${
                    predict.direction > 0
                      ? styles.up
                      : predict.direction < 0
                      ? styles.down
                      : ''
                  }`}
                >
                  {fmtUSD(predict.target_price)}
                </div>
                <div className={styles.subValue}>
                  {predict.target_bps >= 0 ? '+' : ''}
                  {predict.target_bps.toFixed(1)} bps
                </div>
              </div>
              <div className={styles.confRing}>
                <ConfidenceRing value={predict.confidence} />
              </div>
            </div>

            <div className={styles.featureGrid}>
              <Feature label="OFI 5m" value={predict.features.ofi_5m} />
              <Feature
                label="CVD slope"
                value={predict.features.cvd_slope_5m}
                unit="BTC/m"
                precision={2}
              />
              <Feature
                label="Vacuum bias"
                value={predict.features.vacuum_imbalance_5m}
              />
              <Feature
                label="Wall pressure"
                value={predict.features.wall_pressure}
              />
            </div>
          </>
        ) : (
          <div className={styles.empty}>Awaiting data…</div>
        )}
      </section>

      <section className={styles.wallsCard}>
        <div className={styles.cardHead}>
          <div className={styles.cardTitle}>Live walls</div>
          <div className={styles.cardMeta}>{walls.length} tracked</div>
        </div>
        <div className={styles.wallCols}>
          <div className={styles.wallCol}>
            <div className={`${styles.colHead} ${styles.up}`}>Bids</div>
            {bidWalls.length === 0 && (
              <div className={styles.empty}>No significant bid walls</div>
            )}
            {bidWalls.map((w) => (
              <WallRow key={w.id} w={w} />
            ))}
          </div>
          <div className={styles.wallCol}>
            <div className={`${styles.colHead} ${styles.down}`}>Asks</div>
            {askWalls.length === 0 && (
              <div className={styles.empty}>No significant ask walls</div>
            )}
            {askWalls.map((w) => (
              <WallRow key={w.id} w={w} />
            ))}
          </div>
        </div>
      </section>

      <section className={styles.feedCard}>
        <div className={styles.cardHead}>
          <div className={styles.cardTitle}>Vacuum events</div>
          <div className={styles.cardMeta}>{vacuums.length}</div>
        </div>
        {vacuums.length === 0 ? (
          <div className={styles.empty}>No vacuums in the last window.</div>
        ) : (
          <ul className={styles.feedList}>
            {vacuums.slice(0, 12).map((v, i) => (
              <li key={`${v.ts}-${i}`} className={styles.feedItem}>
                <span
                  className={`${styles.dotSm} ${
                    v.side === 'ask' ? styles.up : styles.down
                  }`}
                />
                <div className={styles.feedMain}>
                  <div className={styles.feedTop}>
                    <span className="tnum">{fmtUSD(v.price)}</span>
                    <span className={styles.feedTag}>
                      {v.side === 'ask' ? 'ask pulled · bullish' : 'bid pulled · bearish'}
                    </span>
                  </div>
                  <div className={styles.feedBot}>
                    <span className="tnum">{fmtCompact(v.notional_pulled)}</span>
                    <span className={styles.sep}>·</span>
                    <span>{v.reason}</span>
                    <span className={styles.sep}>·</span>
                    <span className="tnum">{fmtTimeAgo(v.ts)}</span>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <footer className={styles.footer}>
        <span>Binance · BTCUSDT</span>
        <span className={styles.sep}>·</span>
        <span>15-min horizon</span>
      </footer>
    </main>
  );
}

function Feature({
  label,
  value,
  unit,
  precision = 2,
}: {
  label: string;
  value: number;
  unit?: string;
  precision?: number;
}) {
  const pos = value > 0;
  const neg = value < 0;
  return (
    <div className={styles.feature}>
      <div className={styles.featureLabel}>{label}</div>
      <div
        className={`${styles.featureValue} tnum ${
          pos ? styles.up : neg ? styles.down : ''
        }`}
      >
        {value > 0 ? '+' : ''}
        {value.toFixed(precision)}
        {unit ? <span className={styles.featureUnit}> {unit}</span> : null}
      </div>
    </div>
  );
}

function WallRow({ w }: { w: Wall }) {
  return (
    <div className={styles.wallRow}>
      <span className={`${styles.wallPrice} tnum`}>{fmtUSD(w.price)}</span>
      <span className={`${styles.wallSize} tnum`}>
        {fmtCompact(w.notional)}
      </span>
      <span className={styles.wallDist}>
        {Math.abs(w.distance_bps).toFixed(0)} bps
      </span>
    </div>
  );
}

function ConfidenceRing({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(1, value));
  const r = 22;
  const c = 2 * Math.PI * r;
  return (
    <svg width="60" height="60" viewBox="0 0 60 60">
      <circle
        cx="30"
        cy="30"
        r={r}
        stroke="var(--hairline)"
        strokeWidth="3"
        fill="none"
      />
      <circle
        cx="30"
        cy="30"
        r={r}
        stroke="var(--accent)"
        strokeWidth="3"
        fill="none"
        strokeDasharray={`${c * pct} ${c}`}
        strokeLinecap="round"
        transform="rotate(-90 30 30)"
      />
      <text
        x="30"
        y="34"
        textAnchor="middle"
        fontSize="14"
        fill="var(--fg)"
        fontWeight="600"
        className="tnum"
      >
        {Math.round(pct * 100)}
      </text>
    </svg>
  );
}
