use chrono::Utc;
use common::{BookSnapshot, PredictFeatures, Prediction};

use crate::features::Features;

/// 15-minute directional model.
///
/// Combines four signals into a normalized direction score, then projects a
/// price target by scaling 15-min ATR with the "thinness" of the path the
/// price would have to travel. Confidence rises when signals agree.
pub fn predict(book: &BookSnapshot, f: &Features) -> Prediction {
    // Tunable weights — calibrated to give roughly equal influence at typical magnitudes
    let w_ofi = 0.30;
    let w_cvd = 0.25;
    let w_vac = 0.30;
    let w_wall = 0.15;

    // Normalize CVD slope (BTC/min) to ~[-1, 1] by clipping at 50 BTC/min
    let cvd_n = (f.cvd_slope_5m / 50.0).clamp(-1.0, 1.0);
    // OFI and vacuum_imbalance, wall_pressure are already in [-1, 1]
    let ofi_n = f.ofi_5m.clamp(-1.0, 1.0);
    let vac_n = f.vacuum_imbalance_5m.clamp(-1.0, 1.0);
    let wall_n = f.wall_pressure.clamp(-1.0, 1.0);

    let direction_score =
        w_ofi * ofi_n + w_cvd * cvd_n + w_vac * vac_n + w_wall * wall_n;

    // Path thinness: walls remaining above/below dampen movement, vacuums above mid
    // boost up-thinness, vacuums below mid boost down-thinness.
    // Use wall_pressure: positive means more bid depth (thicker below), so up-thinness > down-thinness.
    let thinness_up = (1.0 - wall_n).clamp(0.0, 2.0) / 2.0;   // 0..1
    let thinness_down = (1.0 + wall_n).clamp(0.0, 2.0) / 2.0; // 0..1

    let direction = if direction_score > 0.05 {
        1
    } else if direction_score < -0.05 {
        -1
    } else {
        0
    };

    // Magnitude: ATR scaled by 1.0 (full ATR if all signals max out and path is thin)
    let thin = if direction > 0 {
        thinness_up
    } else if direction < 0 {
        thinness_down
    } else {
        0.5
    };
    let magnitude_bps = f.atr_15m_bps * direction_score.abs().min(1.0) * (0.5 + thin);
    let target_bps = direction as f64 * magnitude_bps;
    let target_price = book.mid * (1.0 + target_bps / 10_000.0);

    // Confidence: alignment of signals + magnitude
    let signs = [
        ofi_n.signum(),
        cvd_n.signum(),
        vac_n.signum(),
        wall_n.signum(),
    ];
    let agree = signs
        .iter()
        .filter(|&&s| s != 0.0 && s.signum() == direction_score.signum())
        .count() as f64;
    let confidence = ((agree / 4.0) * 0.6 + direction_score.abs().min(1.0) * 0.4).min(1.0);

    let label = describe(direction, confidence, &f);

    Prediction {
        ts: Utc::now().timestamp_millis(),
        mid: book.mid,
        direction,
        target_price,
        target_bps,
        confidence,
        horizon_seconds: 15 * 60,
        features: PredictFeatures {
            ofi_5m: f.ofi_5m,
            cvd_slope_5m: f.cvd_slope_5m,
            vacuum_imbalance_5m: f.vacuum_imbalance_5m,
            wall_pressure: f.wall_pressure,
            atr_15m_bps: f.atr_15m_bps,
            thinness_up,
            thinness_down,
            direction_score,
        },
        label,
    }
}

fn describe(direction: i8, conf: f64, _f: &Features) -> String {
    let dir = match direction {
        1 => "Up",
        -1 => "Down",
        _ => "Neutral",
    };
    let band = if conf > 0.7 {
        "high conviction"
    } else if conf > 0.4 {
        "moderate"
    } else {
        "weak"
    };
    format!("{dir} · {band}")
}
