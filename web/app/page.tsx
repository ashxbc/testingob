'use client';

import { useEffect, useRef, useState } from 'react';
import styles from './page.module.css';
import {
  BookSnapshot,
  PredictPayload,
  Thesis,
  VacuumEvent,
  Wall,
  WatchState,
} from '@/lib/types';
import { fmtUSD, fmtCompact, fmtBps, fmtTimeAgo } from '@/lib/format';

interface GhostWall {
  key: string;
  side: 'bid' | 'ask';
  price: number;
  notional: number;
  reason: VacuumEvent['reason'];
  defense_count: number;
  ts: number;
}

export default function Home() {
  const [book, setBook] = useState<BookSnapshot | null>(null);
  const [predict, setPredict] = useState<PredictPayload | null>(null);
  const [walls, setWalls] = useState<Wall[]>([]);
  const [vacuums, setVacuums] = useState<VacuumEvent[]>([]);
  const [ghosts, setGhosts] = useState<GhostWall[]>([]);
  const [connected, setConnected] = useState(false);
  const ghostTimer = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

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

  // Polling fallback — keeps data fresh even when SSE drops (Vercel proxy cuts long connections)
  useEffect(() => {
    const poll = async () => {
      const [b, p, w, v] = await Promise.all([
        fetch('/api/state').then((r) => r.json()).catch(() => null),
        fetch('/api/predict').then((r) => r.json()).catch(() => null),
        fetch('/api/walls').then((r) => r.json()).catch(() => null),
        fetch('/api/vacuums').then((r) => r.json()).catch(() => null),
      ]);
      if (b) setBook(b);
      if (p) setPredict(p);
      if (Array.isArray(w)) setWalls(w);
      if (Array.isArray(v)) {
        setVacuums((prev) => {
          const prevTs = new Set(prev.map((x) => x.ts));
          const fresh = (v as VacuumEvent[]).filter((x) => !prevTs.has(x.ts));
          fresh.forEach((vac) => {
            const key = `${vac.side}-${vac.price}-${vac.ts}`;
            const ghost: GhostWall = {
              key, side: vac.side, price: vac.price,
              notional: vac.notional_pulled, reason: vac.reason,
              defense_count: vac.defense_count, ts: vac.ts,
            };
            setGhosts((prev) => [ghost, ...prev].slice(0, 20));
            const t = setTimeout(() => {
              setGhosts((prev) => prev.filter((g) => g.key !== key));
              ghostTimer.current.delete(key);
            }, 3500);
            ghostTimer.current.set(key, t);
          });
          return [...(v as VacuumEvent[]), ...prev]
            .filter((x, i, arr) => arr.findIndex((y) => y.ts === x.ts) === i)
            .slice(0, 50);
        });
      }
    };
    poll();
    const interval = setInterval(poll, 3000);
    return () => clearInterval(interval);
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
      const v: VacuumEvent = JSON.parse((e as MessageEvent).data);
      setVacuums((prev) => [v, ...prev].slice(0, 50));

      const key = `${v.side}-${v.price}-${v.ts}`;
      const ghost: GhostWall = {
        key,
        side: v.side,
        price: v.price,
        notional: v.notional_pulled,
        reason: v.reason,
        defense_count: v.defense_count,
        ts: v.ts,
      };
      setGhosts((prev) => [ghost, ...prev].slice(0, 20));

      const t = setTimeout(() => {
        setGhosts((prev) => prev.filter((g) => g.key !== key));
        ghostTimer.current.delete(key);
      }, 3500);
      ghostTimer.current.set(key, t);
    });
    return () => es.close();
  }, []);

  const sortedWalls = [...walls].sort(
    (a, b) => Math.abs(a.distance_bps) - Math.abs(b.distance_bps)
  );
  const bidWalls = sortedWalls.filter((w) => w.side === 'bid').slice(0, 5);
  const askWalls = sortedWalls.filter((w) => w.side === 'ask').slice(0, 5);

  const isThesis = predict && predict.kind === 'thesis';
  const isWatching = predict && predict.kind === 'watching';

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

      {isThesis ? (
        <ThesisCard t={predict as Thesis & { kind: 'thesis' }} />
      ) : isWatching ? (
        <WatchingCard w={predict as WatchState & { kind: 'watching' }} />
      ) : (
        <section className={styles.predictCard}>
          <div className={styles.empty}>Awaiting data…</div>
        </section>
      )}

      <section className={styles.wallsCard}>
        <div className={styles.cardHead}>
          <div className={styles.cardTitle}>Live walls</div>
          <div className={styles.cardMeta}>{walls.length} tracked</div>
        </div>
        <div className={styles.wallCols}>
          <div className={styles.wallCol}>
            <div className={`${styles.colHead} ${styles.up}`}>Bids</div>
            {bidWalls.length === 0 && ghosts.filter(g => g.side === 'bid').length === 0 && (
              <div className={styles.empty}>No significant bid walls</div>
            )}
            {bidWalls.map((w) => (
              <WallRow key={w.id} w={w} />
            ))}
            {ghosts.filter(g => g.side === 'bid').map((g) => (
              <GhostRow key={g.key} g={g} />
            ))}
          </div>
          <div className={styles.wallCol}>
            <div className={`${styles.colHead} ${styles.down}`}>Asks</div>
            {askWalls.length === 0 && ghosts.filter(g => g.side === 'ask').length === 0 && (
              <div className={styles.empty}>No significant ask walls</div>
            )}
            {askWalls.map((w) => (
              <WallRow key={w.id} w={w} />
            ))}
            {ghosts.filter(g => g.side === 'ask').map((g) => (
              <GhostRow key={g.key} g={g} />
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
                      {v.side === 'ask'
                        ? 'ask pulled · bullish'
                        : 'bid pulled · bearish'}
                    </span>
                  </div>
                  <div className={styles.feedBot}>
                    <span className="tnum">
                      {fmtCompact(v.notional_pulled)}
                    </span>
                    <span className={styles.sep}>·</span>
                    <span>{v.reason}</span>
                    {v.defense_count > 0 && (
                      <>
                        <span className={styles.sep}>·</span>
                        <span>defended ×{v.defense_count}</span>
                      </>
                    )}
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

function ThesisCard({ t }: { t: Thesis }) {
  const dirUp = t.direction > 0;
  const statusLabel =
    t.status === 'active'
      ? 'Active'
      : t.status === 'filled'
      ? 'Target hit'
      : t.status === 'invalidated'
      ? 'Invalidated'
      : t.status === 'expired'
      ? 'Expired'
      : 'Reversed';
  const remaining = Math.max(0, t.expires_at - Date.now());
  const remainMin = Math.floor(remaining / 60_000);
  const remainSec = Math.floor((remaining % 60_000) / 1000);

  return (
    <section className={styles.predictCard}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>15-minute thesis</div>
        <div
          className={`${styles.confBadge} ${dirUp ? styles.up : styles.down}`}
        >
          {dirUp ? '↑ Up' : '↓ Down'} · {statusLabel}
        </div>
      </div>

      <div className={styles.targetRow}>
        <div>
          <div className={styles.subLabel}>Target</div>
          <div
            className={`${styles.targetPrice} tnum ${
              dirUp ? styles.up : styles.down
            }`}
          >
            {fmtUSD(t.target_price)}
          </div>
          <div className={styles.subValue}>{t.target_reason}</div>
        </div>
        <div className={styles.confRing}>
          <ConfidenceRing value={t.confidence} />
        </div>
      </div>

      <div className={styles.progressBar}>
        <div
          className={`${styles.progressFill} ${
            dirUp ? styles.upBg : styles.downBg
          }`}
          style={{
            width: `${Math.max(0, Math.min(100, t.progress * 100))}%`,
          }}
        />
      </div>
      <div className={styles.progressLabels}>
        <span className="tnum">{fmtUSD(t.mid_at_creation)}</span>
        <span className="tnum">{fmtUSD(t.current_mid)}</span>
        <span className="tnum">{fmtUSD(t.target_price)}</span>
      </div>

      <div className={styles.triggerBox}>
        <div className={styles.subLabel}>Trigger</div>
        <div className={styles.triggerText}>{t.trigger.event}</div>
      </div>

      <div className={styles.killBox}>
        <span>
          Stop <span className="tnum">{fmtUSD(t.stop_price)}</span>
        </span>
        <span className={styles.sep}>·</span>
        <span>
          Expires in{' '}
          <span className="tnum">
            {remainMin}m {remainSec.toString().padStart(2, '0')}s
          </span>
        </span>
      </div>

      <ul className={styles.checklist}>
        {t.checklist.map((c, i) => (
          <li
            key={i}
            className={`${styles.check} ${
              c.passed ? styles.checkPass : styles.checkFail
            }`}
          >
            <span className={styles.checkMark}>{c.passed ? '✓' : '·'}</span>
            <span>{c.label}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function WatchingCard({ w }: { w: WatchState }) {
  return (
    <section className={styles.predictCard}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>15-minute thesis</div>
        <div className={`${styles.confBadge} ${styles.neutral}`}>
          Awaiting trigger
        </div>
      </div>

      <div className={styles.empty} style={{ padding: '4px 0 14px' }}>
        No high-quality wall pull has triggered a thesis. Watching:
      </div>

      <ul className={styles.watchList}>
        {w.watching.map((line, i) => (
          <li key={i} className={styles.watchItem}>
            <span className={styles.watchDot} />
            <span>{line}</span>
          </li>
        ))}
      </ul>

      {w.last_thesis && (
        <div className={styles.lastThesisBox}>
          <div className={styles.subLabel}>Last thesis</div>
          <div className={styles.lastThesisText}>
            {w.last_thesis.direction > 0 ? '↑' : '↓'} target{' '}
            <span className="tnum">{fmtUSD(w.last_thesis.target_price)}</span>{' '}
            — <span>{w.last_thesis.status}</span>
          </div>
        </div>
      )}
    </section>
  );
}

function WallRow({ w }: { w: Wall }) {
  return (
    <div className={styles.wallRow}>
      <span className={`${styles.wallPrice} tnum`}>{fmtUSD(w.price)}</span>
      <span className={`${styles.wallSize} tnum`}>{fmtCompact(w.notional)}</span>
      <span className={styles.wallDist}>
        {Math.abs(w.distance_bps).toFixed(0)} bps
        {w.touches > 0 && <> · ×{w.touches}</>}
      </span>
    </div>
  );
}

function GhostRow({ g }: { g: GhostWall }) {
  const isCancelled = g.reason === 'cancelled' || g.reason === 'mixed';
  const isBullish = g.side === 'ask' && isCancelled;
  const isBearish = g.side === 'bid' && isCancelled;
  const label = isCancelled
    ? g.defense_count > 0
      ? `cancelled · defended ×${g.defense_count}`
      : 'cancelled'
    : 'filled';
  return (
    <div
      className={`${styles.ghostRow} ${
        isBullish
          ? styles.ghostBull
          : isBearish
          ? styles.ghostBear
          : styles.ghostNeutral
      }`}
    >
      <span className={`${styles.wallPrice} tnum`}>{fmtUSD(g.price)}</span>
      <span className={`${styles.wallSize} tnum`}>{fmtCompact(g.notional)}</span>
      <span className={styles.ghostLabel}>{label}</span>
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
