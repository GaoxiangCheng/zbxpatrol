//! 巡检数据模型：serde 序列化后即 `--data-json` / HTTP API 的响应结构（接口契约，只增不改）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------- 基础记录 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRec {
    pub groupid: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostid: String,
    /// 技术主机名
    pub host: String,
    /// 可见名
    pub name: String,
    pub ip: String,
    pub groups: Vec<String>,
    /// Linux / Windows / FreeBSD / macOS / AIX / 其他 / 未知（hosts 列表时填充）
    #[serde(default)]
    pub os_family: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRec {
    pub itemid: String,
    pub hostid: String,
    pub key: String,
    pub name: String,
    /// 0 float / 1 char / 2 log / 3 unsigned / 4 text
    pub value_type: u8,
    pub units: String,
    pub lastvalue: Option<String>,
    pub lastclock: Option<i64>,
}

impl ItemRec {
    pub fn last_f64(&self) -> Option<f64> {
        self.lastvalue.as_deref().and_then(|v| v.trim().parse::<f64>().ok())
    }
    pub fn numeric(&self) -> bool {
        self.value_type == 0 || self.value_type == 3
    }
}

// ---------- 范围 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Scope {
    All,
    Groups(Vec<String>),
    Hosts(Vec<String>),
}

impl Scope {
    pub fn label(&self) -> String {
        match self {
            Scope::All => "全部主机".to_string(),
            Scope::Groups(g) => g.join("+"),
            Scope::Hosts(h) => {
                if h.len() == 1 { h[0].clone() } else { format!("{}台主机", h.len()) }
            }
        }
    }
}

// ---------- 统计 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricStats {
    pub cur: Option<f64>,
    pub avg: Option<f64>,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub unit: String,
    pub count: u64,
    /// history | trend | derived | none
    pub source: String,
    pub missing: bool,
}

impl MetricStats {
    pub fn empty(unit: &str) -> Self {
        MetricStats { unit: unit.into(), missing: true, ..Default::default() }
    }
    pub fn has_avg(&self) -> bool {
        self.avg.is_some()
    }
    /// 对取反型指标（pfree → pused）做 100-x 变换
    pub fn invert(mut self) -> Self {
        for v in [&mut self.cur, &mut self.avg, &mut self.max, &mut self.min] {
            if let Some(x) = v.as_mut() {
                *x = 100.0 - *x;
            }
        }
        self
    }
}

pub fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.1}"),
        None => "—".to_string(),
    }
}

/// 原始采样点（--raw 模式）：一条历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSample {
    pub key: String,
    pub name: String,
    pub clock: i64,
    pub value: f64,
    pub unit: String,
}

// ---------- 分区 / 网卡 / 服务 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskStat {
    pub mount: String,
    pub total_b: Option<f64>,
    pub used_b: Option<f64>,
    pub space: MetricStats,
    pub inode: Option<MetricStats>,
    /// 最小二乘外推的预计满盘天数（仅长区间计算）
    pub forecast_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetIfStat {
    pub ifname: String,
    pub in_mbps: Option<MetricStats>,
    pub out_mbps: Option<MetricStats>,
    pub util_pct: Option<MetricStats>,
    pub in_errors: Option<u64>,
    pub out_errors: Option<u64>,
    pub in_dropped: Option<u64>,
    pub out_dropped: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceCheck {
    pub key: String,
    pub name: String,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StabilityInfo {
    pub reboots: Option<i64>,
    pub time_offset_s: Option<MetricStats>,
    pub zombies: Option<MetricStats>,
    pub fd_util: Option<MetricStats>,
    pub cpu_num: Option<i64>,
    pub load1: Option<MetricStats>,
    pub load5: Option<MetricStats>,
    pub load15: Option<MetricStats>,
    pub icmp_loss: Option<MetricStats>,
    pub icmp_latency: Option<MetricStats>,
    pub cert_min_days: Option<MetricStats>,
    pub oom_events: Option<i64>,
}

// ---------- 问题 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemRec {
    pub eventid: String,
    pub name: String,
    /// 0 未分类 1 信息 2 警告 3 一般 4 高危 5 灾难
    pub severity: u8,
    pub severity_label: String,
    pub clock: i64,
    pub recovered: bool,
    pub acknowledged: bool,
    pub hosts: Vec<String>,
}

// ---------- 评分 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskResult {
    pub score: i64,
    pub level: String,
    pub points: Vec<String>,
}

pub fn risk_level(score: i64) -> &'static str {
    match score {
        0..=39 => "健康",
        40..=59 => "低危",
        60..=74 => "中危",
        75..=89 => "高危",
        _ => "严重",
    }
}

// ---------- 主机巡检结果 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainMetrics {
    pub cpu: MetricStats,
    pub mem: MetricStats,
    pub swap: Option<MetricStats>,
    /// 最满分区（按 avg 排序，缺 avg 用 cur）
    pub disk_max: Option<DiskStat>,
    pub inode_max: Option<DiskStat>,
    pub uptime_days: Option<f64>,
}

/// 主机趋势迷你数据（分桶均值，供火花线/图表渲染）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSpark {
    pub cpu: Vec<Option<f64>>,
    pub mem: Vec<Option<f64>>,
    pub disk: Vec<Option<f64>>,
    /// disk 序列对应的挂载点
    pub disk_mount: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInspection {
    pub host: HostInfo,
    pub os: String,
    /// Linux / Windows / …（由 uname/sw.os 推导）
    pub os_family: String,
    pub available: bool,
    pub metrics: MainMetrics,
    pub disks: Vec<DiskStat>,
    pub nets: Vec<NetIfStat>,
    pub services: Vec<ServiceCheck>,
    pub stability: StabilityInfo,
    pub risk: RiskResult,
    pub problems: Vec<ProblemRec>,
    /// --all-items 时的全量数值指标统计
    pub all_items: Option<BTreeMap<String, MetricStats>>,
    /// --raw 时的原始逐条数据：key → (timestamp, value) 列表
    #[serde(default)]
    pub raw_data: Option<Vec<RawSample>>,
    /// 趋势迷你序列（报表默认生成）
    #[serde(default)]
    pub spark: Option<HostSpark>,
    /// 报表附加的自定义指标统计（--keys / 向导选择）
    #[serde(default)]
    pub extra: Option<BTreeMap<String, MetricStats>>,
    /// 任一核心指标缺失数据（用于退出码 4）
    pub missing_data: bool,
}

// ---------- 报告 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskDist {
    pub healthy: i64,
    pub low: i64,
    pub medium: i64,
    pub high: i64,
    pub critical: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopRisk {
    pub host: String,
    pub score: i64,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Summary {
    pub host_total: i64,
    pub available: i64,
    pub unavailable: i64,
    pub missing_data: i64,
    pub risk_dist: RiskDist,
    pub top_risk: Vec<TopRisk>,
    /// 当前未恢复告警总数（含开始于区间之前的遗留告警）
    pub problem_open: i64,
    /// 其中开始于区间之内的新增告警数
    #[serde(default)]
    pub problem_new_in_range: i64,
    /// 其中开始于区间之前的遗留告警数
    #[serde(default)]
    pub problem_carried_over: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeInfo {
    pub from: i64,
    pub till: i64,
    pub tz: String,
    /// 人类可读区间
    pub human: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportData {
    pub version: String,
    pub generated_at: i64,
    pub range: RangeInfo,
    /// "all" | "group" | "hosts" + 名称
    pub scope_type: String,
    pub scope_names: Vec<String>,
    /// 评分严格度：宽松(基准90) | 标准(基准80) | 严格(基准70)
    pub strictness: String,
    pub summary: Summary,
    pub hosts: Vec<HostInspection>,
    pub problems: Vec<ProblemRec>,
}
