'use client';

import { useEffect, useRef, useState } from 'react';
import styles from './page.module.css';
import {
  BookSnapshot,
  Cluster,
  ClusterSnapshot,
  Liquidation,
  PredictPayload,
  Thesis,
  VacuumEvent,
  Wall,
  WatchState,
} from '@/lib/types';
import { fmtUSD, fmtCompact, fmtBps, fmtTimeAgo } from '@/lib/format';

type GhostReason = VacuumEvent['reason'] | 'reduced';

interface GhostWall {
  key: string;
  side: 'bid' | 'ask';
  price: number;
  notional: number;
  reason: GhostReason;
  defense_count: number;
  ts: number;
}

interface FlashLiq {
  key: string;
  liq: Liquidation;
}

export default function Home() {
  const [book, setBook] = useState<BookSnapshot | null>(null);
  const [predict, setPredict] = useState<PredictPayload | null>(null);
  const [walls, setWalls] = useState<Wall[]>([]);
  const [vacuums, setVacuums] = useState<VacuumEvent[]>([]);
  const [ghosts, setGhosts] = useState<GhostWall[]>([]);
  const [clusters, setClusters] = useState<ClusterSnapshot | null>(null);
  const [liquidations, setLiquidations] = useState<Liquidation[]>([]);
  const [flashLiqs, setFlashLiqs] = useState<FlashLiq[]>([]);
  const [connected, setConnected] = useState(false);
  const ghostTimer = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  const flashTimer = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  const prevWalls = useRef<Wall[]>([]);
  const recentVacuums = useRef<VacuumEvent[]>([]);

  // Initial load
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [b, p, w, v, c, l] = await Promise.all([
        fetch('/api/state').then((r) => r.json()).catch(() => null),
        fetch('/api/predict').then((r) => r.json()).catch(() => null),
        fetch('/api/walls').then((r) => r.json()).catch(() => []),
        fetch('/api/vacuums').then((r) => r.json()).catch(() => []),
        fetch('/api/clusters').then((r) => r.json()).catch(() => null),
        fetch('/api/liquidations').then((r) => r.json()).catch(() => []),
      ]);
      if (cancelled) return;
      if (b) setBook(b);
      if (p) setPredict(p);
      if (Array.isArray(w)) setWalls(w);
      if (Array.isArray(v)) setVacuums(v);
      if (c) setClusters(c);
      if (Array.isArray(l)) setLiquidations(l);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Polling — keeps things fresh + drives wall ghosts via diff
  useEffect(() => {
    const spawnGhost = (wall: Wall, vac: VacuumEvent | undefined) => {
      const key = `ghost-${wall.id}-${Date.now()}`;
      const ghost: GhostWall = {
        key,
        side: wall.side,
        price: wall.price,
        notional: wall.notional,
        reason: vac ? vac.reason : 'reduced',
        defense_count: vac?.defense_count ?? wall.touches,
        ts: Date.now(),
      };
      setGhosts((prev) => [ghost, ...prev].slice(0, 20));
      const t = setTimeout(() => {
        setGhosts((prev) => prev.filter((g) => g.key !== key));
        ghostTimer.current.delete(key);
      }, 4500);
      ghostTimer.current.set(key, t);
    };

    const poll = async () => {
      const [b, p, w, v, c, l] = await Promise.all([
        fetch('/api/state').then((r) => r.json()).catch(() => null),
        fetch('/api/predict').then((r) => r.json()).catch(() => null),
        fetch('/api/walls').then((r) => r.json()).catch(() => null),
        fetch('/api/vacuums').then((r) => r.json()).catch(() => null),
        fetch('/api/clusters').then((r) => r.json()).catch(() => null),
        fetch('/api/liquidations').then((r) => r.json()).catch(() => null),
      ]);
      if (b) setBook(b);
      if (p) setPredict(p);
      if (c) setClusters(c);

      if (Array.isArray(v)) {
        recentVacuums.current = (v as VacuumEvent[]).slice(0, 20);
        setVacuums((prev) => {
          const prevTs = new Set(prev.map((x) => x.ts));
          const fresh = (v as VacuumEvent[]).filter((x) => !prevTs.has(x.ts));
          return [...fresh, ...prev].slice(0, 50);
        });
      }

      if (Array.isArray(l)) {
        setLiquidations((prev) => {
          const prevKeys = new Set(prev.map((x) => `${x.ts}-${x.exchange}-${x.price}`));
          const fresh = (l as Liquidation[]).filter(
            (x) => !prevKeys.has(`${x.ts}-${x.exchange}-${x.price}`)
          );
          fresh.forEach((liq) => {
            const key = `flash-${liq.ts}-${liq.exchange}-${liq.price}-${Math.random()}`;
            setFlashLiqs((prev) => [{ key, liq }, ...prev].slice(0, 6));
            const t = setTimeout(() => {
              setFlashLiqs((prev) => prev.filter((f) => f.key !== key));
              flashTimer.current.delete(key);
            }, 4000);
            flashTimer.current.set(key, t);
          });
          return [...fresh, ...prev].slice(0, 100);
        });
      }

      if (Array.isArray(w)) {
        const newWalls = w as Wall[];
        const newIds = new Set(newWalls.map((x) => x.id));
        const disappeared = prevWalls.current.filter((pw) => !newIds.has(pw.id));
        disappeared.forEach((gone) => {
          const match = recentVacuums.current.find((vac) => vac.wall_id === gone.id);
          spawnGhost(gone, match);
        });
        prevWalls.current = newWalls;
        setWalls(newWalls);
      }
    };

    poll();
    const interval = setInterval(poll, 2000);
    return () => clearInterval(interval);
  }, []);

  // SSE for instant updates (works on direct VPS, may break on Vercel — polling above is fallback)
  useEffect(() => {
    const es = new EventSource('/api/stream');
    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.addEventListener('book', (e) => setBook(JSON.parse((e as MessageEvent).data)));
    es.addEventListener('predict', (e) =>
      setPredict(JSON.parse((e as MessageEvent).data))
    );
    es.addEventListener('clusters', (e) =>
      setClusters(JSON.parse((e as MessageEvent).data))
    );
    es.addEventListener('vacuum', (e) => {
      const v: VacuumEvent = JSON.parse((e as MessageEvent).data);
      setVacuums((prev) => [v, ...prev].slice(0, 50));
      recentVacuums.current = [v, ...recentVacuums.current].slice(0, 20);
    });
    es.addEventListener('liq', (e) => {
      const liq: Liquidation = JSON.parse((e as MessageEvent).data);
      const key = `flash-sse-${liq.ts}-${liq.exchange}-${liq.price}-${Math.random()}`;
      setLiquidations((prev) => [liq, ...prev].slice(0, 100));
      setFlashLiqs((prev) => [{ key, liq }, ...prev].slice(0, 6));
      const t = setTimeout(() => {
        setFlashLiqs((prev) => prev.filter((f) => f.key !== key));
        flashTimer.current.delete(key);
      }, 4000);
      flashTimer.current.set(key, t);
    });
    return () => es.close();
  }, []);

  const isThesis = predict && predict.kind === 'thesis';
  const isWatching = predict && predict.kind === 'watching';

  return (
    <main className={styles.main}>
      <Header connected={connected} />

      <PriceHero book={book} />

      {isThesis ? (
        <ThesisCard t={predict as Thesis & { kind: 'thesis' }} />
      ) : isWatching ? (
        <WatchingCard w={predict as WatchState & { kind: 'watching' }} />
      ) : (
        <section className={styles.card}>
          <div className={styles.empty}>Awaiting data…</div>
        </section>
      )}

      <LiquidationMap clusters={clusters} flashLiqs={flashLiqs} book={book} />

      <WallsCard walls={walls} ghosts={ghosts} />

      <ActivityFeed vacuums={vacuums} liquidations={liquidations} />

      <Footer />
    </main>
  );
}

function Header({ connected }: { connected: boolean }) {
  return (
    <header className={styles.header}>
      <div className={styles.brand}>
        <Crosshair />
        <span className={styles.brandName}>huntr</span>
      </div>
      <div className={styles.symbol}>
        <span>BTC</span>
        <span className={`${styles.dot} ${connected ? styles.live : ''}`} />
      </div>
    </header>
  );
}

function Crosshair() {
  return (
    <svg
      className={styles.logo}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
    >
      <circle cx="12" cy="12" r="9" />
      <line x1="12" y1="2" x2="12" y2="6" />
      <line x1="12" y1="18" x2="12" y2="22" />
      <line x1="2" y1="12" x2="6" y2="12" />
      <line x1="18" y1="12" x2="22" y2="12" />
      <circle cx="12" cy="12" r="1.5" fill="currentColor" />
    </svg>
  );
}

function PriceHero({ book }: { book: BookSnapshot | null }) {
  return (
    <section className={styles.hero}>
      <div className={styles.heroLabel}>BTC · spot</div>
      <div className={`${styles.heroPrice} tnum`}>
        {book ? fmtUSD(book.mid) : '—'}
      </div>
      <div className={styles.heroMeta}>
        <span className="tnum">Bid {book ? fmtUSD(book.best_bid) : '—'}</span>
        <span className={styles.sep}>·</span>
        <span className="tnum">Ask {book ? fmtUSD(book.best_ask) : '—'}</span>
        <span className={styles.sep}>·</span>
        <span className="tnum">Spread {book ? fmtBps(book.spread_bps) : '—'}</span>
      </div>
    </section>
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
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>15-min thesis</div>
        <div
          className={`${styles.confBadge} ${dirUp ? styles.up : styles.down}`}
        >
          {dirUp ? '↑' : '↓'} {dirUp ? 'Long' : 'Short'} · {statusLabel}
        </div>
      </div>

      <div className={styles.targetRow}>
        <div>
          <div className={styles.subLabel}>Target</div>
          <div
            className={`${styles.targetPrice} tnum ${dirUp ? styles.up : styles.down}`}
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
          className={`${styles.progressFill} ${dirUp ? styles.upBg : styles.downBg}`}
          style={{ width: `${Math.max(0, Math.min(100, t.progress * 100))}%` }}
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
          Expires{' '}
          <span className="tnum">
            {remainMin}m {remainSec.toString().padStart(2, '0')}s
          </span>
        </span>
      </div>

      <ul className={styles.checklist}>
        {t.checklist.map((c, i) => (
          <li
            key={i}
            className={`${styles.check} ${c.passed ? styles.checkPass : styles.checkFail}`}
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
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>15-min thesis</div>
        <div className={`${styles.confBadge} ${styles.neutral}`}>Awaiting trigger</div>
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

// ----------------- Liquidation Map -----------------

function LiquidationMap({
  clusters,
  flashLiqs,
  book,
}: {
  clusters: ClusterSnapshot | null;
  flashLiqs: FlashLiq[];
  book: BookSnapshot | null;
}) {
  if (!clusters || !book) {
    return (
      <section className={styles.card}>
        <div className={styles.cardHead}>
          <div className={styles.cardTitle}>Liquidation map</div>
          <div className={styles.cardMeta}>—</div>
        </div>
        <div className={styles.empty}>Awaiting cluster data…</div>
      </section>
    );
  }

  const mid = book.mid;
  const above = clusters.clusters
    .filter((c) => c.bucket > mid)
    .sort((a, b) => a.bucket - b.bucket)
    .slice(0, 8);
  const below = clusters.clusters
    .filter((c) => c.bucket < mid)
    .sort((a, b) => b.bucket - a.bucket)
    .slice(0, 8);

  // Reverse `above` so the highest is at the top
  const aboveTopFirst = [...above].reverse();

  return (
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>Liquidation map</div>
        <div className={styles.cardMeta}>
          <span className={styles.down}>shorts ${fmtCompactBare(clusters.short_total)}</span>
          <span className={styles.sep}>·</span>
          <span className={styles.up}>longs ${fmtCompactBare(clusters.long_total)}</span>
        </div>
      </div>

      <div className={styles.liqMap}>
        {aboveTopFirst.length === 0 && below.length === 0 ? (
          <div className={styles.empty}>No clusters within ±2% of price.</div>
        ) : (
          <>
            <div className={styles.liqSide}>
              {aboveTopFirst.map((c) => (
                <ClusterRow key={`a-${c.bucket}`} c={c} />
              ))}
              {aboveTopFirst.length === 0 && (
                <div className={styles.liqEmpty}>no shorts above</div>
              )}
            </div>

            <div className={styles.midLine}>
              <span className="tnum">{fmtUSD(mid)}</span>
              <span className={styles.midDot} />
              <span className={styles.midLabel}>mid</span>
            </div>

            <div className={styles.liqSide}>
              {below.map((c) => (
                <ClusterRow key={`b-${c.bucket}`} c={c} />
              ))}
              {below.length === 0 && (
                <div className={styles.liqEmpty}>no longs below</div>
              )}
            </div>
          </>
        )}

        {flashLiqs.length > 0 && (
          <div className={styles.flashStack}>
            {flashLiqs.slice(0, 3).map((f) => (
              <div
                key={f.key}
                className={`${styles.flashLiq} ${
                  f.liq.side === 'long' ? styles.flashLong : styles.flashShort
                }`}
              >
                <span className={styles.flashEx}>{f.liq.exchange}</span>
                <span className="tnum">{fmtUSD(f.liq.price)}</span>
                <span className={styles.flashSide}>
                  {f.liq.side === 'long' ? 'long liq' : 'short liq'}
                </span>
                <span className={`tnum ${styles.flashSize}`}>
                  ${fmtCompactBare(f.liq.notional)}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function ClusterRow({ c }: { c: Cluster }) {
  const isShort = c.side === 'short';
  const tone = isShort ? styles.down : styles.up;
  const bg = isShort ? styles.barShort : styles.barLong;
  const pct = Math.max(8, Math.round(c.strength * 100));
  return (
    <div className={styles.clusterRow}>
      <div className={`${styles.clusterPrice} tnum ${tone}`}>{fmtUSD(c.bucket)}</div>
      <div className={styles.barWrap}>
        <div
          className={`${styles.bar} ${bg}`}
          style={{ width: `${pct}%`, opacity: 0.35 + c.strength * 0.65 }}
        />
      </div>
      <div className={`${styles.clusterSize} tnum`}>
        ${fmtCompactBare(c.total_notional)}
      </div>
      <div className={styles.clusterMeta}>
        {c.exchanges.length}× · {Math.abs(c.distance_bps).toFixed(0)}bps
      </div>
    </div>
  );
}

function fmtCompactBare(v: number): string {
  if (!isFinite(v)) return '—';
  if (v >= 1e9) return `${(v / 1e9).toFixed(2)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(2)}M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(0)}K`;
  return v.toFixed(0);
}

// ----------------- Walls -----------------

function WallsCard({ walls, ghosts }: { walls: Wall[]; ghosts: GhostWall[] }) {
  const sortedWalls = [...walls].sort(
    (a, b) => Math.abs(a.distance_bps) - Math.abs(b.distance_bps)
  );
  const bidWalls = sortedWalls.filter((w) => w.side === 'bid').slice(0, 5);
  const askWalls = sortedWalls.filter((w) => w.side === 'ask').slice(0, 5);

  return (
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>Walls</div>
        <div className={styles.cardMeta}>{walls.length} tracked</div>
      </div>
      <div className={styles.wallCols}>
        <div className={styles.wallCol}>
          <div className={`${styles.colHead} ${styles.up}`}>Bids</div>
          {bidWalls.length === 0 && ghosts.filter((g) => g.side === 'bid').length === 0 && (
            <div className={styles.empty}>—</div>
          )}
          {bidWalls.map((w) => (
            <WallRow key={w.id} w={w} />
          ))}
          {ghosts
            .filter((g) => g.side === 'bid')
            .map((g) => (
              <GhostRow key={g.key} g={g} />
            ))}
        </div>
        <div className={styles.wallCol}>
          <div className={`${styles.colHead} ${styles.down}`}>Asks</div>
          {askWalls.length === 0 && ghosts.filter((g) => g.side === 'ask').length === 0 && (
            <div className={styles.empty}>—</div>
          )}
          {askWalls.map((w) => (
            <WallRow key={w.id} w={w} />
          ))}
          {ghosts
            .filter((g) => g.side === 'ask')
            .map((g) => (
              <GhostRow key={g.key} g={g} />
            ))}
        </div>
      </div>
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
  let toneClass = styles.ghostNeutral;
  let title = 'unknown';
  let detail = '';

  if (g.reason === 'cancelled') {
    toneClass = g.side === 'ask' ? styles.ghostBull : styles.ghostBear;
    title = 'Pulled';
    detail =
      g.defense_count > 0
        ? `owner cancelled · defended ×${g.defense_count}`
        : 'owner cancelled — no trades hit it';
  } else if (g.reason === 'mixed') {
    toneClass = g.side === 'ask' ? styles.ghostBull : styles.ghostBear;
    title = 'Mostly pulled';
    detail = 'partially cancelled, partially traded';
  } else if (g.reason === 'filled') {
    toneClass = styles.ghostNeutral;
    title = 'Filled';
    detail = 'traded through — buyers/sellers ate it';
  } else if (g.reason === 'reduced') {
    toneClass = styles.ghostReduced;
    title = 'Faded';
    detail = 'slowly trimmed below tracking threshold';
  }

  return (
    <div className={`${styles.ghostRow} ${toneClass}`}>
      <div className={styles.ghostTop}>
        <span className={`${styles.wallPrice} tnum`}>{fmtUSD(g.price)}</span>
        <span className={`${styles.wallSize} tnum`}>{fmtCompact(g.notional)}</span>
        <span className={styles.ghostTitle}>{title}</span>
      </div>
      <div className={styles.ghostDetail}>{detail}</div>
    </div>
  );
}

// ----------------- Activity feed -----------------

type FeedItem =
  | { kind: 'vacuum'; ts: number; v: VacuumEvent }
  | { kind: 'liq'; ts: number; l: Liquidation };

function ActivityFeed({
  vacuums,
  liquidations,
}: {
  vacuums: VacuumEvent[];
  liquidations: Liquidation[];
}) {
  const items: FeedItem[] = [
    ...vacuums.map((v) => ({ kind: 'vacuum' as const, ts: v.ts, v })),
    ...liquidations.map((l) => ({ kind: 'liq' as const, ts: l.ts, l })),
  ]
    .sort((a, b) => b.ts - a.ts)
    .slice(0, 20);

  return (
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <div className={styles.cardTitle}>Activity</div>
        <div className={styles.cardMeta}>
          {vacuums.length} pulls · {liquidations.length} liqs
        </div>
      </div>
      {items.length === 0 ? (
        <div className={styles.empty}>No recent events.</div>
      ) : (
        <ul className={styles.feedList}>
          {items.map((it, i) =>
            it.kind === 'vacuum' ? (
              <FeedVacuum key={`v-${it.v.ts}-${i}`} v={it.v} />
            ) : (
              <FeedLiq key={`l-${it.l.ts}-${it.l.exchange}-${i}`} l={it.l} />
            )
          )}
        </ul>
      )}
    </section>
  );
}

function FeedVacuum({ v }: { v: VacuumEvent }) {
  const isBull = v.side === 'ask' && (v.reason === 'cancelled' || v.reason === 'mixed');
  const isBear = v.side === 'bid' && (v.reason === 'cancelled' || v.reason === 'mixed');
  return (
    <li className={styles.feedItem}>
      <span
        className={`${styles.dotSm} ${isBull ? styles.up : isBear ? styles.down : styles.neutral}`}
      />
      <div className={styles.feedMain}>
        <div className={styles.feedTop}>
          <span className="tnum">{fmtUSD(v.price)}</span>
          <span className={styles.feedTag}>
            {v.side === 'ask' ? 'ask' : 'bid'} {v.reason}
          </span>
        </div>
        <div className={styles.feedBot}>
          <span className="tnum">{fmtCompact(v.notional_pulled)}</span>
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
  );
}

function FeedLiq({ l }: { l: Liquidation }) {
  const isLong = l.side === 'long';
  return (
    <li className={styles.feedItem}>
      <span
        className={`${styles.dotSm} ${isLong ? styles.down : styles.up}`}
        title={isLong ? 'long liquidated' : 'short liquidated'}
      />
      <div className={styles.feedMain}>
        <div className={styles.feedTop}>
          <span className="tnum">{fmtUSD(l.price)}</span>
          <span className={styles.feedTag}>
            {l.exchange} · {isLong ? 'long liq' : 'short liq'}
          </span>
        </div>
        <div className={styles.feedBot}>
          <span className="tnum">{fmtCompact(l.notional)}</span>
          <span className={styles.sep}>·</span>
          <span className="tnum">{fmtTimeAgo(l.ts)}</span>
        </div>
      </div>
    </li>
  );
}

function Footer() {
  return (
    <footer className={styles.footer}>
      <span>spot · Binance</span>
      <span className={styles.sep}>·</span>
      <span>liqs · Binance · Bybit · OKX</span>
      <span className={styles.sep}>·</span>
      <span>15-min horizon</span>
    </footer>
  );
}

function ConfidenceRing({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(1, value));
  const r = 22;
  const c = 2 * Math.PI * r;
  return (
    <svg width="60" height="60" viewBox="0 0 60 60">
      <circle cx="30" cy="30" r={r} stroke="var(--hairline)" strokeWidth="3" fill="none" />
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
