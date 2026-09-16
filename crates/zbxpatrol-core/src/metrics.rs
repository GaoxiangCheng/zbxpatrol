//! 聚合算法：history 全量统计、trend 加权、计数器差分速率、重启检测、满盘预测、时间偏移。

use crate::types::MetricStats;
use crate::zabbix::TrendRow;

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub clock: i64,
    pub value: f64,
}

/// history 样本 → (avg, max, min, n)
pub fn agg_history(samples: &[Sample]) -> Option<(f64, f64, f64, u64)> {
    if samples.is_empty() {
        return None;
    }
    let mut sum = 0.0;
    let mut max = f64::MIN;
    let mut min = f64::MAX;
    for s in samples {
        sum += s.value;
        max = max.max(s.value);
        min = min.min(s.value);
    }
    let n = samples.len() as u64;
    Some((sum / n as f64, max, min, n))
}

/// trend 小时记录 → 加权 avg = Σ(avgᵢ·numᵢ)/Σnumᵢ，max/max，min/min
pub fn agg_trends(rows: &[TrendRow]) -> Option<(f64, f64, f64, u64)> {
    let mut sum_wa = 0.0;
    let mut sum_w = 0.0;
    let mut max = f64::MIN;
    let mut min = f64::MAX;
    let mut total = 0u64;
    for r in rows {
        let w = r.num.max(0.0);
        sum_wa += r.avg * w;
        sum_w += w;
        max = max.max(r.max);
        min = min.min(r.min);
        total += w as u64;
    }
    if sum_w <= 0.0 {
        return None;
    }
    Some((sum_wa / sum_w, max, min, total))
}

/// 累计计数器 → 速率样本（Δvalue/Δt），单位与原值每秒一致
pub fn counter_rates(samples: &[Sample]) -> Vec<Sample> {
    let mut rates = Vec::new();
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dt = b.clock - a.clock;
        let dv = b.value - a.value;
        // 计数器回绕（重启）或异常跳变时跳过该窗口
        if dt > 0 && dv >= 0.0 && dt < 3600 {
            rates.push(Sample { clock: b.clock, value: dv / dt as f64 });
        }
    }
    rates
}

/// 计数器差分总和（区间内新增量，如错包/丢包个数）；回绕（重启清零）后的首个窗口跳过
pub fn counter_delta(samples: &[Sample]) -> Option<u64> {
    if samples.len() < 2 {
        return None;
    }
    let mut total = 0.0f64;
    let mut after_reset = false;
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.value < a.value {
            after_reset = true;
        } else {
            if !after_reset {
                total += b.value - a.value;
            }
            after_reset = false;
        }
    }
    Some(total as u64)
}

/// 重启次数：boottime 序列容差去重计数 - 1（每次开机 boottime 唯一）
///
/// 部分系统 agent 上报的 boottime 会随时钟校准每日漂移数秒，精确去重会把
/// 漂移当成多次重启；真实重启的 boottime 跳变远大于漂移，故按相邻差值容差去重。
pub fn reboots_from_boottime(samples: &[Sample]) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    const DRIFT_TOLERANCE_SECS: i64 = 300;
    let mut vals: Vec<i64> = samples.iter().map(|s| s.value as i64).collect();
    vals.sort_unstable();
    let mut boots = 1usize;
    for w in vals.windows(2) {
        if w[1] - w[0] > DRIFT_TOLERANCE_SECS {
            boots += 1;
        }
    }
    Some(boots as i64 - 1)
}

/// 重启次数（无 boottime 时）：uptime 序列向下跳变次数
pub fn reboots_from_uptime(samples: &[Sample]) -> Option<i64> {
    if samples.len() < 2 {
        return None;
    }
    let mut n = 0;
    for w in samples.windows(2) {
        if w[1].value < w[0].value {
            n += 1;
        }
    }
    Some(n)
}

/// 时间偏移：max |localtime值 - 采样clock|（秒）
pub fn time_offset(samples: &[Sample]) -> Option<MetricStats> {
    if samples.is_empty() {
        return None;
    }
    let offs: Vec<f64> = samples.iter().map(|s| (s.value - s.clock as f64).abs()).collect();
    let (avg, max, min, n) = agg_history(
        &offs.iter().map(|v| Sample { clock: 0, value: *v }).collect::<Vec<_>>(),
    )?;
    Some(MetricStats {
        cur: offs.last().copied(),
        avg: Some(avg),
        max: Some(max),
        min: Some(min),
        unit: "s".into(),
        count: n,
        source: "derived".into(),
        missing: false,
    })
}

/// 最小二乘斜率（单位：value/天）
pub fn slope_per_day(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 3 {
        return None;
    }
    let t0 = samples[0].clock as f64;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let n = samples.len() as f64;
    for s in samples {
        let x = (s.clock as f64 - t0) / 86400.0;
        sx += x;
        sy += s.value;
        sxx += x * x;
        sxy += x * s.value;
    }
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-12 {
        return None;
    }
    Some((n * sxy - sx * sy) / denom)
}

/// 预计满盘天数：当前使用率按斜率线性外推到 100%
pub fn days_to_full(cur: f64, slope: f64) -> Option<f64> {
    if slope <= 1e-9 {
        return None; // 无增长
    }
    let d = (100.0 - cur) / slope;
    if d.is_finite() && d > 0.0 {
        Some(d)
    } else {
        None
    }
}

/// 时间分桶：把样本按 [from,till] 均分为 buckets 段，每段取均值（无样本为 None）
pub fn bucket_series(samples: &[Sample], from: i64, till: i64, buckets: usize) -> Vec<Option<f64>> {
    if buckets == 0 || till <= from {
        return Vec::new();
    }
    let span = (till - from) / buckets as i64 + 1;
    let mut sums = vec![0.0f64; buckets];
    let mut counts = vec![0u32; buckets];
    for s in samples {
        let idx = ((s.clock - from) as usize / span as usize).min(buckets - 1);
        sums[idx] += s.value;
        counts[idx] += 1;
    }
    (0..buckets)
        .map(|i| {
            if counts[i] > 0 {
                Some(sums[i] / counts[i] as f64)
            } else {
                None
            }
        })
        .collect()
}

/// 序列整体取反（pfree → pused）
pub fn invert_series(series: &[Option<f64>]) -> Vec<Option<f64>> {
    series.iter().map(|v| v.map(|x| 100.0 - x)).collect()
}

/// 简单通配符匹配（* → 任意，其余字面），用于 --keys / query
pub fn wildcard_match(pattern: &str, key: &str) -> bool {
    if pattern == key {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    let re = regex::Regex::new(
        &format!(
            "^{}$",
            regex::escape(pattern).replace("\\*", ".*").replace("\\?", ".")
        ),
    )
    .expect("valid wildcard regex");
    re.is_match(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(clock: i64, value: f64) -> Sample {
        Sample { clock, value }
    }

    #[test]
    fn agg_history_basic() {
        let v = vec![s(1, 10.0), s(2, 20.0), s(3, 30.0)];
        let (avg, max, min, n) = agg_history(&v).unwrap();
        assert!((avg - 20.0).abs() < 1e-9);
        assert_eq!(max, 30.0);
        assert_eq!(min, 10.0);
        assert_eq!(n, 3);
        assert!(agg_history(&[]).is_none());
    }

    #[test]
    fn reboots_boottime_drift_is_not_reboot() {
        // 实测场景：boottime 每天漂移约 1 秒（24h 内 933→946），不应计为重启
        let drift: Vec<Sample> = (0..24).map(|i| s(i * 3600, 1721454933.0 + i as f64)).collect();
        assert_eq!(reboots_from_boottime(&drift), Some(0));
    }

    #[test]
    fn reboots_boottime_real_reboot() {
        // 真实重启：boottime 跳变数月
        let v = vec![s(1, 1_700_000_000.0), s(2, 1_700_000_001.0), s(3, 1_730_000_000.0)];
        assert_eq!(reboots_from_boottime(&v), Some(1));
        // 漂移累计但相邻差值始终小于容差 → 仍为 0（与桶起点比较会在第 24 天误报）
        let creeping: Vec<Sample> = (0..300).map(|i| s(i * 3600, 1721454933.0 + i as f64)).collect();
        assert_eq!(reboots_from_boottime(&creeping), Some(0));
        assert_eq!(reboots_from_boottime(&[]), None);
    }

    #[test]
    fn reboots_uptime_drops() {
        let v = vec![s(1, 5000.0), s(2, 6000.0), s(3, 100.0), s(4, 200.0)];
        assert_eq!(reboots_from_uptime(&v), Some(1));
    }

    #[test]
    fn agg_trends_weighted() {
        let rows = vec![
            TrendRow { itemid: "1".into(), clock: 1, num: 60.0, min: 1.0, avg: 10.0, max: 20.0 },
            TrendRow { itemid: "1".into(), clock: 2, num: 30.0, min: 2.0, avg: 40.0, max: 50.0 },
        ];
        let (avg, max, min, n) = agg_trends(&rows).unwrap();
        // (10*60 + 40*30) / 90 = 1800/90 = 20
        assert!((avg - 20.0).abs() < 1e-9);
        assert_eq!(max, 50.0);
        assert_eq!(min, 1.0);
        assert_eq!(n, 90);
    }

    #[test]
    fn counter_rate_and_delta() {
        // 每 10s +100 → 10/s
        let v = vec![s(0, 0.0), s(10, 100.0), s(20, 200.0), s(30, 250.0)];
        let rates = counter_rates(&v);
        let (avg, _, _, _) = agg_history(&rates).unwrap();
        assert!((avg - 25.0 / 3.0).abs() < 1e-9, "avg={avg}");
        assert_eq!(counter_delta(&v), Some(250));
        // 回绕跳过
        let wrap = vec![s(0, 900.0), s(10, 950.0), s(20, 5.0), s(30, 60.0)];
        assert_eq!(counter_delta(&wrap), Some(50));
    }

    #[test]
    fn reboot_detection() {
        // 真实重启：boottime 跳变 30 天；秒级/分钟级波动属于漂移不计
        let boot = vec![s(1, 1.7e9), s(2, 1.7e9), s(3, 1.7e9 + 30.0 * 86400.0), s(4, 1.7e9 + 30.0 * 86400.0)];
        assert_eq!(reboots_from_boottime(&boot), Some(1));
        let drift = vec![s(1, 1.7e9), s(2, 1.7e9 + 5.0), s(3, 1.7e9 + 20.0), s(4, 1.7e9 + 20.0)];
        assert_eq!(reboots_from_boottime(&drift), Some(0));
        let up = vec![s(1, 50000.0), s(2, 50060.0), s(3, 10.0), s(4, 70.0)];
        assert_eq!(reboots_from_uptime(&up), Some(1));
    }

    #[test]
    fn offset_and_forecast() {
        let lt = vec![s(1000, 1002.0), s(2000, 2005.0)];
        let st = time_offset(&lt).unwrap();
        assert_eq!(st.max.unwrap() as i64, 5);
        // 每天 +1%：50% → 100% 需 50 天
        let mut v = Vec::new();
        for d in 0..10 {
            v.push(s(d * 86400, 50.0 + d as f64));
        }
        let slope = slope_per_day(&v).unwrap();
        assert!((slope - 1.0).abs() < 1e-6);
        assert!((days_to_full(50.0, slope).unwrap() - 50.0).abs() < 1e-6);
        assert!(days_to_full(50.0, 0.0).is_none());
    }

    #[test]
    fn wildcard() {
        assert!(wildcard_match("net.if*", "net.if.in[\"eth0\"]"));
        assert!(wildcard_match("system.cpu.util", "system.cpu.util"));
        assert!(!wildcard_match("system.cpu.util", "system.cpu.util[,idle]"));
        assert!(wildcard_match("vfs.fs*inode*", "vfs.fs.dependent.inode[/,pfree]"));
    }

    #[test]
    fn bucketing_series() {
        // 4 桶，每桶 10s：样本落在前两桶
        let samples = vec![s(0, 10.0), s(5, 20.0), s(12, 30.0), s(100, 40.0)];
        let b = bucket_series(&samples, 0, 40, 4);
        assert_eq!(b.len(), 4);
        assert_eq!(b[0], Some(15.0)); // (10+20)/2
        assert_eq!(b[1], Some(30.0));
        assert_eq!(b[2], None);
        let inv = invert_series(&b);
        assert_eq!(inv[0], Some(85.0));
        assert_eq!(inv[2], None);
    }
}
