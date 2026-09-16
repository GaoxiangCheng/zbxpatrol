//! 风险评分：内置默认规则 + 三档严格度（宽松/标准/严格）+ TOML 声明式规则（存在即替换内置）。
//!
//! 严格度以「基准」为核心：宽松=90%、标准=80%、严格=70%。基准即高危线，
//! 全部百分比类阈值随基准平移（宽松 +10 / 标准 0 / 严格 -10，相对标准档）；
//! 事件类规则（重启/OOM/僵尸/时间偏移/证书/告警）不随基准缩放。
//! 自定义 TOML 规则存在时整体替换内置评分，不受严格度影响。

use crate::patrol_config::ScoringRule;
use crate::types::{risk_level, HostInspection, RiskResult};

/// 评分严格度三档
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strictness {
    /// 宽松：基准 90%
    Loose,
    /// 标准：基准 80%（默认，等于 v1.0 行为）
    #[default]
    Standard,
    /// 严格：基准 70%
    Strict,
}

impl Strictness {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "loose" | "宽松" => Some(Strictness::Loose),
            "standard" | "std" | "标准" => Some(Strictness::Standard),
            "strict" | "严格" => Some(Strictness::Strict),
            _ => None,
        }
    }
    /// 基准（= 高危线）
    pub fn base(self) -> f64 {
        match self {
            Strictness::Loose => 90.0,
            Strictness::Standard => 80.0,
            Strictness::Strict => 70.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Strictness::Loose => "宽松",
            Strictness::Standard => "标准",
            Strictness::Strict => "严格",
        }
    }
    /// CPU 阶梯：严重/高危/中危/低危（高危=基准-5；封顶 98 保持可达）
    pub fn util_levels(self) -> [f64; 4] {
        let b = self.base();
        [ (b + 5.0).min(98.0), b - 5.0, b - 15.0, b - 30.0 ]
    }
    /// 内存阶梯（内存常态偏高，整体比 CPU 高一档；封顶 98）
    pub fn mem_levels(self) -> [f64; 4] {
        let b = self.base();
        [ (b + 10.0).min(98.0), b, b - 10.0, b - 20.0 ]
    }
    /// 磁盘/inode 阶梯（磁盘整体偏高一档；封顶 98）
    pub fn disk_levels(self) -> [f64; 4] {
        let b = self.base();
        [ (b + 10.0).min(98.0), b + 5.0, b, b - 5.0 ]
    }
    /// CPU 峰值阈值
    pub fn cpu_peak(self) -> f64 {
        (self.base() + 15.0).min(99.0)
    }
    /// swap 阈值
    pub fn swap_thr(self) -> f64 {
        self.base() - 30.0
    }
    /// 带宽/fd 使用率阈值
    pub fn pct_thr(self) -> f64 {
        self.base()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Scorer {
    custom: Option<Vec<ScoringRule>>,
    mode: Strictness,
}

impl Scorer {
    pub fn new(custom: Option<Vec<ScoringRule>>) -> Self {
        Scorer { custom, mode: Strictness::Standard }
    }

    pub fn with_strictness(custom: Option<Vec<ScoringRule>>, mode: Strictness) -> Self {
        // 自定义规则整体替换内置评分，严格度不生效
        Scorer { custom, mode }
    }

    pub fn mode(&self) -> Strictness {
        self.mode
    }

    pub fn score(&self, h: &HostInspection) -> RiskResult {
        match &self.custom {
            Some(rules) => score_custom(h, rules),
            None => score_builtin(h, self.mode),
        }
    }
}

struct Acc {
    points: i64,
    msgs: Vec<String>,
    floored: bool,
}

impl Acc {
    fn add(&mut self, pts: i64, msg: String) {
        self.points += pts;
        self.msgs.push(format!("[+{pts}] {msg}"));
    }
    fn floor(&mut self, msg: String) {
        self.floored = true;
        self.msgs.push(format!("[严重] {msg}"));
    }
    fn result(self) -> RiskResult {
        let mut score = self.points.min(100);
        if self.floored {
            score = score.max(95);
        }
        RiskResult { score, level: risk_level(score).to_string(), points: self.msgs }
    }
}

/// 阶梯评分：命中最高档（levels 由高到低）
fn tier(acc: &mut Acc, value: f64, levels: [f64; 4], base: [i64; 4], label: &str) {
    const NAMES: [&str; 4] = ["严重", "高危", "中危", "低危"];
    for i in 0..4 {
        if value >= levels[i] {
            acc.add(
                base[i],
                format!("{label} 使用率 {value:.1}%，达到{}阈值 {}%", NAMES[i], levels[i]),
            );
            return;
        }
    }
}

fn score_builtin(h: &HostInspection, mode: Strictness) -> RiskResult {
    let mut acc = Acc { points: 0, msgs: Vec::new(), floored: false };

    // 可用性：直接严重
    if !h.available {
        acc.floor("主机不可达（Agent 无响应）".into());
    }
    if let Some(svc) = h.services.iter().find(|s| !s.ok) {
        acc.floor(format!("服务探测失败：{}（{}）", svc.name, svc.key));
    }

    // CPU
    if let Some(avg) = h.metrics.cpu.avg {
        tier(&mut acc, avg, mode.util_levels(), [60, 45, 30, 15], "CPU");
    }
    if let Some(max) = h.metrics.cpu.max {
        if max >= mode.cpu_peak() {
            acc.add(10, format!("CPU 峰值 {max:.1}%（≥{}%）", mode.cpu_peak()));
        }
    }
    if let (Some(load), Some(cores)) = (h.stability.load1.as_ref(), h.stability.cpu_num) {
        if cores > 0 {
            let limit = cores as f64 * 1.5;
            let avg = load.avg.unwrap_or(0.0);
            if avg > limit {
                acc.add(10, format!("1 分钟负载 {avg:.2} 超过核数×1.5（{limit:.1}）"));
            }
        }
    }

    // 内存
    if let Some(avg) = h.metrics.mem.avg {
        tier(&mut acc, avg, mode.mem_levels(), [60, 45, 30, 15], "内存");
    }
    if let Some(avg) = h.metrics.swap.as_ref().and_then(|s| s.avg) {
        if avg >= mode.swap_thr() {
            acc.add(10, format!("Swap 平均使用率 {avg:.1}%（≥{}%）", mode.swap_thr()));
        }
    }

    // 磁盘：空间 + inode 阶梯、满盘预测
    if let Some(d) = h.metrics.disk_max.as_ref() {
        let v = d.space.avg.or(d.space.cur).unwrap_or(0.0);
        tier(&mut acc, v, mode.disk_levels(), [60, 45, 30, 15],
            &format!("磁盘最满分区 {}", d.mount));
        if let Some(days) = d.forecast_days {
            if days <= 90.0 {
                acc.add(15, format!("分区 {} 按当前增速预计 {:.0} 天内占满", d.mount, days));
            }
        }
    }
    if let Some(i) = h.metrics.inode_max.as_ref() {
        let v = i.space.avg.or(i.space.cur).unwrap_or(0.0);
        tier(&mut acc, v, mode.disk_levels(), [60, 45, 30, 15],
            &format!("inode 最满分区 {}", i.mount));
    }

    // 网络
    if let Some(loss) = h.stability.icmp_loss.as_ref().and_then(|s| s.avg) {
        if loss > 0.0 {
            acc.add(10, format!("ICMP 平均丢包率 {loss:.1}%"));
        }
    }
    let err_total: u64 = h.nets.iter().filter_map(|n| n.out_errors.or(n.in_errors)).sum();
    if err_total > 0 {
        acc.add(5, format!("区间内网卡错包新增 {err_total} 个"));
    }
    if let Some(util) = h
        .nets
        .iter()
        .filter_map(|n| n.util_pct.as_ref())
        .filter_map(|s| s.max)
        .fold(None::<f64>, |acc, v| Some(match acc { Some(a) => a.max(v), None => v }))
    {
        if util >= mode.pct_thr() {
            acc.add(10, format!("网卡带宽峰值利用率 {util:.0}%（≥{}%）", mode.pct_thr()));
        }
    }
    if let Some(fd) = h.stability.fd_util.as_ref().and_then(|s| s.cur) {
        if fd >= mode.pct_thr() {
            acc.add(10, format!("文件描述符使用率 {fd:.0}%（≥{}%）", mode.pct_thr()));
        }
    }

    // 稳定性
    let n = h.stability.reboots.unwrap_or(0);
    if n > 0 {
        acc.add(n * 15, format!("区间内发生 {n} 次重启"));
    }
    if h.stability.oom_events.unwrap_or(0) > 0 {
        acc.add(10, format!("区间内发生 {} 次 OOM 事件", h.stability.oom_events.unwrap_or(0)));
    }
    if let Some(z) = h.stability.zombies.as_ref().and_then(|s| s.cur) {
        if z > 10.0 {
            acc.add(5, format!("僵尸进程 {z:.0} 个（>10）"));
        }
    }
    if let Some(off) = h.stability.time_offset_s.as_ref().and_then(|s| s.max) {
        if off > 30.0 {
            acc.add(10, format!("系统时间偏移最大 {off:.0} 秒（>30s）"));
        }
    }
    if let Some(cert) = h.stability.cert_min_days.as_ref().and_then(|s| s.cur) {
        if cert < 30.0 {
            acc.add(15, format!("证书剩余 {cert:.0} 天（<30 天）"));
        }
    }

    // 告警
    let open_high = h.problems.iter().filter(|p| !p.recovered && p.severity >= 4).count();
    if open_high > 0 {
        let pts = (open_high as i64 * 5).min(20);
        acc.add(pts, format!("{open_high} 个未恢复的高危/灾难告警"));
    }

    acc.result()
}

fn score_custom(h: &HostInspection, rules: &[ScoringRule]) -> RiskResult {
    let mut acc = Acc { points: 0, msgs: Vec::new(), floored: false };
    for r in rules {
        if let Some(v) = resolve_path(h, &r.metric) {
            let hit = match r.op.as_str() {
                "gt" => v > r.value,
                "gte" => v >= r.value,
                "lt" => v < r.value,
                "lte" => v <= r.value,
                "eq" => (v - r.value).abs() < 1e-9,
                _ => false,
            };
            if hit {
                if r.floor {
                    acc.floor(r.message.replace("{value}", &format!("{v:.1}")));
                    continue;
                }
                let pts = if r.per_count { (r.points as f64 * v.max(1.0)) as i64 } else { r.points };
                let msg = if r.message.is_empty() {
                    format!("{} {} {}", r.metric, r.op, r.value)
                } else {
                    r.message.replace("{value}", &format!("{v:.1}"))
                };
                acc.add(pts, msg);
            }
        }
    }
    acc.result()
}

/// 自定义评分规则的指标路径：name.(cur|avg|max|min)
fn resolve_path(h: &HostInspection, path: &str) -> Option<f64> {
    let (name, field) = match path.split_once('.') {
        Some((n, f)) => (n, f),
        None => (path, "avg"),
    };
    let pick = |s: &crate::types::MetricStats| match field {
        "cur" => s.cur,
        "max" => s.max,
        "min" => s.min,
        _ => s.avg,
    };
    match name {
        "cpu" => pick(&h.metrics.cpu),
        "mem" => pick(&h.metrics.mem),
        "swap" => h.metrics.swap.as_ref().and_then(pick),
        "disk" => h.metrics.disk_max.as_ref().and_then(|d| pick(&d.space)),
        "inode" => h.metrics.inode_max.as_ref().and_then(|d| pick(&d.space)),
        "load1" => h.stability.load1.as_ref().and_then(pick),
        "zombies" => h.stability.zombies.as_ref().and_then(pick),
        "offset" => h.stability.time_offset_s.as_ref().and_then(|s| s.max),
        "fd" => h.stability.fd_util.as_ref().and_then(|s| s.cur),
        "cert" => h.stability.cert_min_days.as_ref().and_then(pick),
        "icmp_loss" => h.stability.icmp_loss.as_ref().and_then(pick),
        "reboots" => h.stability.reboots.map(|v| v as f64),
        "oom" => h.stability.oom_events.map(|v| v as f64),
        "problems" => Some(h.problems.iter().filter(|p| !p.recovered && p.severity >= 4).count() as f64),
        "availability" => Some(if h.available { 1.0 } else { 0.0 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn host() -> HostInspection {
        HostInspection {
            host: HostInfo {
                hostid: "1".into(), host: "h1".into(), name: "h1".into(),
                ip: "1.1.1.1".into(), groups: vec![], os_family: String::new(),
            },
            os: "Linux".into(),
            os_family: "Linux".into(),
            available: true,
            metrics: MainMetrics {
                cpu: MetricStats { avg: Some(20.0), max: Some(40.0), ..Default::default() },
                mem: MetricStats { avg: Some(30.0), ..Default::default() },
                swap: None, disk_max: None, inode_max: None, uptime_days: Some(10.0),
            },
            disks: vec![], nets: vec![], services: vec![],
            stability: StabilityInfo::default(),
            risk: RiskResult::default(),
            problems: vec![], all_items: None, spark: None, extra: None, raw_data: None, missing_data: false,
        }
    }

    #[test]
    fn healthy_host_zero() {
        let h = host();
        let r = Scorer::new(None).score(&h);
        assert_eq!(r.score, 0);
        assert_eq!(r.level, "健康");
    }

    #[test]
    fn cpu_mem_severe_capped() {
        let mut h = host();
        h.metrics.cpu.avg = Some(90.0);
        h.metrics.cpu.max = Some(99.0);
        h.metrics.mem.avg = Some(92.0);
        let r = Scorer::new(None).score(&h);
        assert_eq!(r.score, 100);
        assert_eq!(r.level, "严重");
        assert!(r.points.iter().any(|p| p.contains("CPU")));
        assert!(r.points.iter().any(|p| p.contains("峰值")));
    }

    #[test]
    fn unavailable_floors_at_95() {
        let mut h = host();
        h.available = false;
        let r = Scorer::new(None).score(&h);
        assert_eq!(r.score, 95);
        assert_eq!(r.level, "严重");
    }

    #[test]
    fn reboot_and_alert_weights() {
        let mut h = host();
        h.stability.reboots = Some(3);
        h.problems = vec![ProblemRec {
            eventid: "1".into(), name: "x".into(), severity: 4,
            severity_label: "高危".into(), clock: 0, recovered: false,
            acknowledged: false, hosts: vec![],
        }];
        let r = Scorer::new(None).score(&h);
        assert_eq!(r.score, 50); // 45(3次重启×15) + 5(告警)
        assert_eq!(r.level, "低危");
    }

    #[test]
    fn custom_rules_replace_builtin() {
        let rules = vec![ScoringRule {
            metric: "cpu.avg".into(), op: "gte".into(), value: 50.0,
            points: 30, floor: false, message: "CPU {value}%".into(), per_count: false,
        }];
        let mut h = host();
        h.metrics.cpu.avg = Some(60.0);
        h.metrics.mem.avg = Some(95.0); // 内置规则不应再生效
        let r = Scorer::new(Some(rules)).score(&h);
        assert_eq!(r.score, 30);
    }

    #[test]
    fn custom_floor_rule() {
        let rules = vec![ScoringRule {
            metric: "availability".into(), op: "eq".into(), value: 0.0,
            points: 95, floor: true, message: "主机不可达".into(), per_count: false,
        }];
        let mut h = host();
        h.available = false;
        let r = Scorer::new(Some(rules)).score(&h);
        assert_eq!(r.score, 95);
    }

    #[test]
    fn strictness_thresholds_match_spec() {
        // 标准（基准 80）= v1.0 既有行为，逐项对照
        let s = Strictness::Standard;
        assert_eq!(s.base(), 80.0);
        assert_eq!(s.util_levels(), [85.0, 75.0, 65.0, 50.0]);
        assert_eq!(s.mem_levels(), [90.0, 80.0, 70.0, 60.0]);
        assert_eq!(s.disk_levels(), [90.0, 85.0, 80.0, 75.0]);
        assert_eq!(s.cpu_peak(), 95.0);
        assert_eq!(s.swap_thr(), 50.0);
        assert_eq!(s.pct_thr(), 80.0);
        // 宽松（基准 90）：整体放宽 10 个点，封顶 98/99
        let l = Strictness::Loose;
        assert_eq!(l.base(), 90.0);
        assert_eq!(l.util_levels(), [95.0, 85.0, 75.0, 60.0]);
        assert_eq!(l.mem_levels(), [98.0, 90.0, 80.0, 70.0]);
        assert_eq!(l.disk_levels(), [98.0, 95.0, 90.0, 85.0]);
        assert_eq!(l.cpu_peak(), 99.0);
        assert_eq!(l.swap_thr(), 60.0);
        assert_eq!(l.pct_thr(), 90.0);
        // 严格（基准 70）：整体收紧 10 个点
        let t = Strictness::Strict;
        assert_eq!(t.base(), 70.0);
        assert_eq!(t.util_levels(), [75.0, 65.0, 55.0, 40.0]);
        assert_eq!(t.mem_levels(), [80.0, 70.0, 60.0, 50.0]);
        assert_eq!(t.disk_levels(), [80.0, 75.0, 70.0, 65.0]);
        assert_eq!(t.cpu_peak(), 85.0);
        assert_eq!(t.swap_thr(), 40.0);
        assert_eq!(t.pct_thr(), 70.0);
    }

    #[test]
    fn strictness_changes_score() {
        // 内存 72%：标准=中危(30) 宽松=低危(15) 严格=高危(45)
        let mut h = host();
        h.metrics.mem.avg = Some(72.0);
        let std = Scorer::with_strictness(None, Strictness::Standard).score(&h);
        let loose = Scorer::with_strictness(None, Strictness::Loose).score(&h);
        let strict = Scorer::with_strictness(None, Strictness::Strict).score(&h);
        assert_eq!(std.score, 30);
        assert_eq!(loose.score, 15);
        assert_eq!(strict.score, 45);
        assert_eq!(strict.level, "低危");
    }

    #[test]
    fn strictness_parse_cn_en() {
        assert_eq!(Strictness::parse("宽松"), Some(Strictness::Loose));
        assert_eq!(Strictness::parse("loose"), Some(Strictness::Loose));
        assert_eq!(Strictness::parse("标准"), Some(Strictness::Standard));
        assert_eq!(Strictness::parse("std"), Some(Strictness::Standard));
        assert_eq!(Strictness::parse("严格"), Some(Strictness::Strict));
        assert_eq!(Strictness::parse("unknown"), None);
    }
}
