//! 巡检编排：范围解析 → 并发逐主机（发现/分类/取数/聚合）→ 问题关联 → 评分 → 汇总。
//! CLI 与 HTTP serve 共用本模块。

use crate::discovery::{classify_host, resolve_hosts};
use crate::errors::Result;
use crate::metrics::*;
use crate::patrol_config::PatrolToml;
use crate::rules::CompiledRules;
use crate::scoring::Scorer;
use crate::timerange::TimeRange;
use crate::types::*;
use crate::zabbix::{TrendRow, ZabbixClient};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub struct InspectOptions {
    pub all_items: bool,
    pub patrol: PatrolToml,
    pub strictness: crate::scoring::Strictness,
    /// 生成趋势迷你序列（火花线/图表用）
    pub charts: bool,
    /// 附加自定义指标 key（支持通配；统计进 HostInspection.extra）
    pub extra_keys: Vec<String>,
    /// 导出原始逐条数据（CPU/内存/磁盘等核心指标的历史采样值）
    pub raw: bool,
}

#[derive(Debug)]
pub struct InspectOutcome {
    pub data: ReportData,
    /// 任一主机存在数据缺失（退出码 4）
    pub missing: bool,
}

pub type ProgressFn = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// 全量巡检入口
pub async fn run_inspection(
    client: Arc<ZabbixClient>,
    scope: &Scope,
    range: &TimeRange,
    opts: &InspectOptions,
    concurrency: usize,
    progress: Option<ProgressFn>,
) -> Result<InspectOutcome> {
    let hosts = resolve_hosts(&client, scope).await?;
    let rules = Arc::new(CompiledRules::load(&opts.patrol.metric_rules()));
    let total = hosts.len();

    // 并发逐主机巡检（JoinSet，保持原顺序）
    let mut results: Vec<(usize, HostInfo, std::result::Result<HostInspection, crate::errors::PatrolError>)> =
        Vec::new();
    let mut done = 0usize;
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for (idx, host) in hosts.into_iter().enumerate() {
        let client = client.clone();
        let rules = rules.clone();
        let range = range.clone();
        let all_items = opts.all_items;
        let charts = opts.charts;
        let extra_keys = opts.extra_keys.clone();
        let raw = opts.raw;
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            let r = inspect_host(client, &host, &range, &rules, all_items, charts, extra_keys, raw).await;
            (idx, host, r)
        });
    }
    while let Some(joined) = set.join_next().await {
        if let Ok(item) = joined {
            done += 1;
            if let Some(cb) = &progress {
                cb(done, total);
            }
            results.push(item);
        }
    }
    results.sort_by_key(|(idx, _, _)| *idx);

    // 问题告警：当前未恢复（任意开始时间）∪ 区间内发生过（含已恢复），按 eventid 去重后按主机名关联
    let hostids: Vec<String> = results.iter().map(|(_, h, _)| h.hostid.clone()).collect();
    let open = client.problems_open(Some(&hostids)).await?;
    let in_range = client.problems_in_range(range.from, range.till, Some(&hostids)).await?;
    let mut problems = open;
    for p in in_range {
        if !problems.iter().any(|x| x.eventid == p.eventid) {
            problems.push(p);
        }
    }
    problems.sort_by(|a, b| b.severity.cmp(&a.severity).then(b.clock.cmp(&a.clock)));
    let mut by_host: HashMap<String, Vec<ProblemRec>> = HashMap::new();
    for p in &problems {
        for hn in &p.hosts {
            by_host.entry(hn.clone()).or_default().push(p.clone());
        }
    }

    // 评分（含问题加权；三档严格度）
    let scorer = Scorer::with_strictness(
        opts.patrol.scoring_rules().map(|s| s.to_vec()),
        opts.strictness,
    );
    let mut inspected: Vec<HostInspection> = Vec::new();
    let mut missing = false;
    for (_, _, r) in results {
        match r {
            Ok(mut hi) => {
                // 停用触发器/主机的问题只展示，不参与评分
                hi.problems = by_host
                    .remove(&hi.host.host)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| !p.disabled)
                    .collect();
                if hi.host_disabled {
                    // 停用主机不参与评分
                    hi.risk = crate::types::RiskResult {
                        score: 0,
                        level: "停用".into(),
                        points: vec![],
                    };
                } else {
                    hi.risk = scorer.score(&hi);
                }
                if hi.missing_data {
                    missing = true;
                }
                inspected.push(hi);
            }
            Err(e) => {
                // 单台失败不中断整份报表（退出码 4）
                tracing::warn!("主机巡检失败：{e}");
                missing = true;
            }
        }
    }

    let data = build_report(inspected, problems, scope, range, opts.strictness);
    Ok(InspectOutcome { data, missing })
}

fn build_report(
    hosts: Vec<HostInspection>,
    problems: Vec<ProblemRec>,
    scope: &Scope,
    range: &TimeRange,
    strictness: crate::scoring::Strictness,
) -> ReportData {
    let mut summary = Summary { host_total: hosts.len() as i64, ..Default::default() };
    for h in &hosts {
        if h.available {
            summary.available += 1;
        } else {
            summary.unavailable += 1;
        }
        if h.missing_data {
            summary.missing_data += 1;
        }
        match h.risk.level.as_str() {
            "停用" => summary.host_disabled += 1,
            "健康" => summary.risk_dist.healthy += 1,
            "低危" => summary.risk_dist.low += 1,
            "中危" => summary.risk_dist.medium += 1,
            "高危" => summary.risk_dist.high += 1,
            _ => summary.risk_dist.critical += 1,
        }
    }
    summary.problem_open =
        problems.iter().filter(|p| !p.recovered && !p.disabled).count() as i64;
    summary.problem_new_in_range = problems
        .iter()
        .filter(|p| !p.recovered && !p.disabled && p.clock >= range.from)
        .count() as i64;
    summary.problem_carried_over = summary.problem_open - summary.problem_new_in_range;
    let mut top: Vec<&HostInspection> = hosts.iter().collect();
    top.sort_by_key(|h| std::cmp::Reverse(h.risk.score));
    summary.top_risk = top
        .into_iter()
        .take(10)
        .map(|h| TopRisk { host: h.host.host.clone(), score: h.risk.score, level: h.risk.level.clone() })
        .collect();

    let (scope_type, scope_names) = match scope {
        Scope::All => ("all".to_string(), vec![]),
        Scope::Groups(g) => ("group".to_string(), g.clone()),
        Scope::Hosts(h) => ("hosts".to_string(), h.clone()),
    };
    ReportData {
        version: crate::VERSION.to_string(),
        generated_at: chrono::Utc::now().timestamp(),
        range: RangeInfo {
            from: range.from,
            till: range.till,
            tz: range.tz.name().to_string(),
            human: range.fmt_human(),
        },
        scope_type,
        scope_names,
        strictness: format!("{}（基准 {}%）", strictness.label(), strictness.base()),
        summary,
        hosts,
        problems,
    }
}

// ---------- 取数与聚合 ----------

/// 时间区间 ≤1 天优先 history，否则优先 trend；preferred 为空自动回退另一种
fn prefer_history(range: &TimeRange) -> bool {
    range.days() <= 1.0
}

async fn fetch_history_map(
    client: &ZabbixClient,
    items: &[&ItemRec],
    range: &TimeRange,
) -> Result<HashMap<String, Vec<Sample>>> {
    let mut map: HashMap<String, Vec<Sample>> = HashMap::new();
    for vt in [0u8, 3u8] {
        let ids: Vec<String> = items
            .iter()
            .filter(|i| i.value_type == vt)
            .map(|i| i.itemid.clone())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let rows = client.history_get(&ids, vt, range.from, range.till).await?;
        for (itemid, clock, value) in rows {
            map.entry(itemid).or_default().push(Sample { clock, value });
        }
    }
    Ok(map)
}

async fn fetch_trend_map(
    client: &ZabbixClient,
    items: &[&ItemRec],
    range: &TimeRange,
) -> Result<HashMap<String, Vec<TrendRow>>> {
    let ids: Vec<String> = items.iter().map(|i| i.itemid.clone()).collect();
    let mut map: HashMap<String, Vec<TrendRow>> = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let rows = client.trend_get(&ids, range.from, range.till).await?;
    for r in rows {
        map.entry(r.itemid.clone()).or_default().push(r);
    }
    Ok(map)
}

/// 一组 item → 每项统计（含 cur 与 source 标注）
pub async fn aggregate_items(
    client: &ZabbixClient,
    items: &[&ItemRec],
    range: &TimeRange,
) -> Result<HashMap<String, MetricStats>> {
    let mut out: HashMap<String, MetricStats> = HashMap::new();
    if items.is_empty() {
        return Ok(out);
    }
    // 优先来源：≤1 天 history，否则 trend
    let hist_map = if prefer_history(range) {
        Some(fetch_history_map(client, items, range).await?)
    } else {
        None
    };
    let trend_map = if prefer_history(range) {
        None
    } else {
        Some(fetch_trend_map(client, items, range).await?)
    };
    let mut missing_ids: Vec<&ItemRec> = Vec::new();
    for it in items {
        let st = if let Some(hist) = hist_map.as_ref() {
            hist.get(&it.itemid)
                .map(|rows| stats_from_history(it.last_f64(), &it.units, rows))
        } else if let Some(trends) = trend_map.as_ref() {
            trends.get(&it.itemid).map(|rows| stats_from_trends(it.last_f64(), &it.units, rows))
        } else {
            None
        };
        match st {
            Some(s) => {
                out.insert(it.itemid.clone(), s);
            }
            None => missing_ids.push(it),
        }
    }
    // 回退另一种来源
    if !missing_ids.is_empty() {
        if prefer_history(range) {
            let trends = fetch_trend_map(client, &missing_ids, range).await?;
            for it in &missing_ids {
                let st = trends
                    .get(&it.itemid)
                    .map(|rows| stats_from_trends(it.last_f64(), &it.units, rows))
                    .unwrap_or_else(|| empty_stats(it));
                out.insert(it.itemid.clone(), st);
            }
        } else {
            let hist = fetch_history_map(client, &missing_ids, range).await?;
            for it in &missing_ids {
                let st = hist
                    .get(&it.itemid)
                    .map(|rows| stats_from_history(it.last_f64(), &it.units, rows))
                    .unwrap_or_else(|| empty_stats(it));
                out.insert(it.itemid.clone(), st);
            }
        }
    }
    Ok(out)
}

fn empty_stats(it: &ItemRec) -> MetricStats {
    MetricStats::empty(&it.units)
}

/// 取原始样本序列（preferred 来源，缺数据回退另一种；供图表/火花线用）
async fn fetch_samples(
    client: &ZabbixClient,
    items: &[&ItemRec],
    range: &TimeRange,
) -> Result<HashMap<String, Vec<Sample>>> {
    let mut out: HashMap<String, Vec<Sample>> = HashMap::new();
    if items.is_empty() {
        return Ok(out);
    }
    let fill_from_hist = |rows: Vec<(String, i64, f64)>, map: &mut HashMap<String, Vec<Sample>>| {
        for (itemid, clock, value) in rows {
            map.entry(itemid).or_default().push(Sample { clock, value });
        }
    };
    if prefer_history(range) {
        for vt in [0u8, 3u8] {
            let ids: Vec<String> = items.iter().filter(|i| i.value_type == vt).map(|i| i.itemid.clone()).collect();
            if ids.is_empty() { continue; }
            let rows = client.history_get(&ids, vt, range.from, range.till).await?;
            fill_from_hist(rows, &mut out);
        }
        // 回退 trend
        let missing: Vec<&ItemRec> = items.iter().filter(|i| !out.contains_key(&i.itemid)).copied().collect();
        if !missing.is_empty() {
            let tmap = fetch_trend_map(client, &missing, range).await?;
            for it in &missing {
                if let Some(rows) = tmap.get(&it.itemid) {
                    out.insert(it.itemid.clone(), rows.iter().map(|r| Sample { clock: r.clock, value: r.avg }).collect());
                }
            }
        }
    } else {
        let tmap = fetch_trend_map(client, items, range).await?;
        for it in items {
            if let Some(rows) = tmap.get(&it.itemid) {
                out.insert(it.itemid.clone(), rows.iter().map(|r| Sample { clock: r.clock, value: r.avg }).collect());
            }
        }
        let missing: Vec<&ItemRec> = items.iter().filter(|i| !out.contains_key(&i.itemid)).copied().collect();
        if !missing.is_empty() {
            for vt in [0u8, 3u8] {
                let ids: Vec<String> = missing.iter().filter(|i| i.value_type == vt).map(|i| i.itemid.clone()).collect();
                if ids.is_empty() { continue; }
                let rows = client.history_get(&ids, vt, range.from, range.till).await?;
                fill_from_hist(rows, &mut out);
            }
        }
    }
    for v in out.values_mut() {
        v.sort_by_key(|s| s.clock);
    }
    Ok(out)
}

fn stats_from_history(cur: Option<f64>, unit: &str, samples: &[Sample]) -> MetricStats {
    match agg_history(samples) {
        Some((avg, max, min, n)) => MetricStats {
            cur,
            avg: Some(avg),
            max: Some(max),
            min: Some(min),
            unit: unit.to_string(),
            count: n,
            source: "history".into(),
            missing: false,
        },
        None => MetricStats::empty(unit),
    }
}

fn stats_from_trends(cur: Option<f64>, unit: &str, rows: &[TrendRow]) -> MetricStats {
    match agg_trends(rows) {
        Some((avg, max, min, n)) => MetricStats {
            cur,
            avg: Some(avg),
            max: Some(max),
            min: Some(min),
            unit: unit.to_string(),
            count: n,
            source: "trend".into(),
            missing: false,
        },
        None => MetricStats::empty(unit),
    }
}

// ---------- 单主机巡检 ----------

#[allow(clippy::too_many_arguments)]
async fn inspect_host(
    client: Arc<ZabbixClient>,
    host: &HostInfo,
    range: &TimeRange,
    rules: &CompiledRules,
    all_items: bool,
    charts: bool,
    extra_keys: Vec<String>,
    raw: bool,
) -> Result<HostInspection> {
    let items = client.get_items(&host.hostid).await?;
    // 停用监控项不参与指标采集与评分（全量明细 sheet 仍保留展示）
    let enabled_items: Vec<_> = items.iter().filter(|i| i.status != "1").cloned().collect();
    let c = classify_host(&enabled_items, rules);

    // ---- 主指标（preferred 来源聚合）----
    let mut main: Vec<&ItemRec> = Vec::new();
    for i in [&c.cpu_util, &c.mem_util, &c.mem_pavail, &c.swap_util, &c.swap_pfree,
               &c.load1, &c.load5, &c.load15, &c.zombies, &c.proc_num,
               &c.icmp_loss, &c.icmp_sec, &c.cert_days].into_iter().flatten() {
                   main.push(i);
               }
    for d in &c.disks {
        if let Some(i) = &d.pused { main.push(i); }
        if let Some(i) = &d.inode_pused { main.push(i); }
        if let Some(i) = &d.inode_pfree { main.push(i); }
    }
    for n in &c.nets {
        if let Some(i) = &n.in_bytes { main.push(i); }
        if let Some(i) = &n.out_bytes { main.push(i); }
    }
    let main_stats = aggregate_items(&client, &main, range).await?;

    let get = |opt: &Option<ItemRec>| -> Option<MetricStats> {
        opt.as_ref().and_then(|i| main_stats.get(&i.itemid).cloned()).filter(|s| !s.missing)
    };
    let cpu = get(&c.cpu_util).unwrap_or_else(|| MetricStats::empty("%"));
    let mem = get(&c.mem_util)
        .or_else(|| get(&c.mem_pavail).map(|s| s.invert()))
        .unwrap_or_else(|| MetricStats::empty("%"));
    let swap = get(&c.swap_util).or_else(|| get(&c.swap_pfree).map(|s| s.invert()));
    let load1 = get(&c.load1);
    let load5 = get(&c.load5);
    let load15 = get(&c.load15);
    let zombies = get(&c.zombies);
    let icmp_loss = get(&c.icmp_loss);
    let icmp_latency = get(&c.icmp_sec);
    let cert_days = get(&c.cert_days);

    // ---- 磁盘 ----
    let mut disks: Vec<DiskStat> = Vec::new();
    for d in &c.disks {
        let space = d
            .pused
            .as_ref()
            .and_then(|i| main_stats.get(&i.itemid).cloned())
            .filter(|s| !s.missing)
            .unwrap_or_else(|| MetricStats::empty("%"));
        let inode = d
            .inode_pused
            .as_ref()
            .and_then(|i| main_stats.get(&i.itemid).cloned())
            .filter(|s| !s.missing)
            .or_else(|| {
                d.inode_pfree
                    .as_ref()
                    .and_then(|i| main_stats.get(&i.itemid).cloned())
                    .filter(|s| !s.missing)
                    .map(|s| s.invert())
            });
        disks.push(DiskStat {
            mount: d.mount.clone(),
            total_b: d.total.as_ref().and_then(|i| i.last_f64()),
            used_b: d.used.as_ref().and_then(|i| i.last_f64()),
            space,
            inode,
            forecast_days: None,
        });
    }
    // 最满分区（avg 优先，回退 cur）
    let pick_key = |d: &DiskStat| d.space.avg.or(d.space.cur).unwrap_or(f64::MIN);
    let disk_max = disks.iter().max_by(|a, b| pick_key(a).total_cmp(&pick_key(b))).cloned();
    let disk_max_mount = disk_max.as_ref().map(|d| d.mount.clone());
    let inode_mounts: Vec<DiskStat> = disks
        .iter()
        .filter(|d| d.inode.is_some())
        .map(|d| DiskStat {
            mount: d.mount.clone(),
            total_b: None,
            used_b: None,
            space: d.inode.clone().unwrap(),
            inode: None,
            forecast_days: None,
        })
        .collect();
    let inode_max = inode_mounts
        .iter()
        .max_by(|a, b| pick_key(a).total_cmp(&pick_key(b)))
        .cloned();

    // 满盘预测（长区间，用 trend/样本做回归）
    let mut disk_max = disk_max;
    if range.days() > 7.0 {
        if let Some(dm) = disk_max.as_mut() {
            if let Some(pused_item) = c.disks.iter().find(|d| d.mount == dm.mount).and_then(|d| d.pused.as_ref()) {
                let trends = fetch_trend_map(&client, &[pused_item], range).await?;
                if let Some(rows) = trends.get(&pused_item.itemid) {
                    let samples: Vec<Sample> = rows.iter().map(|r| Sample { clock: r.clock, value: r.avg }).collect();
                    if let Some(slope) = slope_per_day(&samples) {
                        dm.forecast_days = days_to_full(dm.space.cur.unwrap_or(0.0), slope);
                    }
                }
            }
        }
    }

    // ---- 网卡 ----
    let mut nets: Vec<NetIfStat> = Vec::new();
    for n in &c.nets {
        let speed_bps = n.speed.as_ref().and_then(|i| i.last_f64());
        let scale = |stats: &MetricStats, k: f64, unit: &str| -> MetricStats {
            let f = |v: f64| v * k;
            MetricStats {
                cur: stats.cur.map(f),
                avg: stats.avg.map(f),
                max: stats.max.map(f),
                min: stats.min.map(f),
                unit: unit.into(),
                count: stats.count,
                source: stats.source.clone(),
                missing: stats.missing,
            }
        };
        // units 含 bps：模板已预处理为速率（bps）→ ÷1e6 得 Mbps；
        // 否则为累计字节计数器 → 差分速率（B/s）→ ×8/1e6
        let detect_rate = |i: &ItemRec| i.units.contains("bps");
        let in_stats = n.in_bytes.as_ref().and_then(|i| main_stats.get(&i.itemid).cloned()).filter(|s| !s.missing);
        let out_stats = n.out_bytes.as_ref().and_then(|i| main_stats.get(&i.itemid).cloned()).filter(|s| !s.missing);
        let in_mbps = match (n.in_bytes.as_ref(), in_stats) {
            (Some(item), Some(st)) => {
                if detect_rate(item) {
                    Some(scale(&st, 1.0 / 1e6, "Mbps"))
                } else {
                    let rated = rate_stats(&client, item, range, st).await;
                    Some(scale(&rated, 8.0 / 1e6, "Mbps"))
                }
            }
            _ => None,
        };
        let out_mbps = match (n.out_bytes.as_ref(), out_stats) {
            (Some(item), Some(st)) => {
                if detect_rate(item) {
                    Some(scale(&st, 1.0 / 1e6, "Mbps"))
                } else {
                    let rated = rate_stats(&client, item, range, st).await;
                    Some(scale(&rated, 8.0 / 1e6, "Mbps"))
                }
            }
            _ => None,
        };
        // 带宽利用率（Mbps / speed）
        let util_pct = if let (Some(sp), Some(st)) = (speed_bps, in_mbps.as_ref().or(out_mbps.as_ref())) {
            let sp_mbps = sp / 1e6;
            if sp_mbps > 0.0 {
                let f = |v: f64| v / sp_mbps * 100.0;
                Some(MetricStats {
                    cur: st.cur.map(f),
                    avg: st.avg.map(f),
                    max: st.max.map(f),
                    min: st.min.map(f),
                    unit: "%".into(),
                    count: st.count,
                    source: st.source.clone(),
                    missing: st.missing,
                })
            } else {
                None
            }
        } else {
            None
        };
        // 错包/丢包：计数器差分（用 history）
        nets.push(NetIfStat {
            ifname: n.ifname.clone(),
            in_mbps,
            out_mbps,
            util_pct,
            in_errors: counter_delta_of(&client, &n.in_err, range).await?,
            out_errors: counter_delta_of(&client, &n.out_err, range).await?,
            in_dropped: counter_delta_of(&client, &n.in_drop, range).await?,
            out_dropped: counter_delta_of(&client, &n.out_drop, range).await?,
        });
    }

    // ---- 稳定性 ----
    // 重启：boottime 容差去重 + uptime 跳变双估计取小者；uptime 全程大于窗口时长则不可能重启
    let reboots = if let Some(up) = &c.uptime {
        let hist = fetch_history_map(&client, &[up], range).await?;
        let up_samples = hist.get(&up.itemid);
        let uptime_min = up_samples.and_then(|s| s.iter().map(|x| x.value).reduce(f64::min));
        if let Some(umin) = uptime_min {
            if umin > (range.till - range.from) as f64 {
                // 窗口内 uptime 始终大于窗口本身 → 期间不可能发生过重启
                Some(0)
            } else if let Some(bt) = &c.boottime {
                let hist = fetch_history_map(&client, &[bt], range).await?;
                hist.get(&bt.itemid).and_then(|s| reboots_from_boottime(s))
            } else {
                up_samples.and_then(|s| reboots_from_uptime(s))
            }
        } else if let Some(bt) = &c.boottime {
            let hist = fetch_history_map(&client, &[bt], range).await?;
            hist.get(&bt.itemid).and_then(|s| reboots_from_boottime(s))
        } else {
            None
        }
    } else if let Some(bt) = &c.boottime {
        let hist = fetch_history_map(&client, &[bt], range).await?;
        hist.get(&bt.itemid).and_then(|s| reboots_from_boottime(s))
    } else {
        None
    };
    let time_offset_s = if let Some(lt) = &c.localtime {
        let hist = fetch_history_map(&client, &[lt], range).await?;
        hist.get(&lt.itemid).and_then(|s| time_offset(s))
    } else {
        None
    };
    let fd_util = match (&c.openfiles, &c.maxfiles) {
        (Some(of), Some(mf)) => match (of.last_f64(), mf.last_f64()) {
            (Some(o), Some(m)) if m > 0.0 => Some(MetricStats {
                cur: Some(o / m * 100.0),
                unit: "%".into(),
                source: "derived".into(),
                ..Default::default()
            }),
            _ => None,
        },
        _ => None,
    };
    let uptime_days = c.uptime.as_ref().and_then(|i| i.last_f64()).map(|v| v / 86400.0);
    let cpu_num = c.cpu_num.as_ref().and_then(|i| i.last_f64()).map(|v| v as i64);

    // ---- 可用性 ----
    let available = c
        .agent_avail
        .as_ref()
        .and_then(|i| i.last_f64())
        .map(|v| v == 1.0)
        .unwrap_or(true);

    // ---- 服务探测 ----
    let services: Vec<ServiceCheck> = c
        .services
        .iter()
        .map(|i| ServiceCheck {
            key: i.key.clone(),
            name: i.name.clone(),
            ok: i.last_f64().map(|v| v == 1.0).unwrap_or(false),
        })
        .collect();

    // ---- OS ----
    let (os_family, os) = crate::discovery::os_family_of(&items);

    // ---- 趋势迷你序列（火花线）----
    let spark = if charts {
        const SPARK_BUCKETS: usize = 48;
        let disk_item = disk_max_mount
            .as_ref()
            .and_then(|m| c.disks.iter().find(|d| &d.mount == m))
            .and_then(|d| d.pused.clone());
        let mem_item = c.mem_util.clone().or_else(|| c.mem_pavail.clone());
        let mut wanted: Vec<ItemRec> = Vec::new();
        if let Some(i) = &c.cpu_util { wanted.push(i.clone()); }
        if let Some(i) = &mem_item { wanted.push(i.clone()); }
        if let Some(i) = &disk_item { wanted.push(i.clone()); }
        let refs: Vec<&ItemRec> = wanted.iter().collect();
        match fetch_samples(&client, &refs, range).await {
            Ok(series) => {
                let bucket = |item: &Option<ItemRec>| -> Vec<Option<f64>> {
                    match item {
                        Some(i) => series
                            .get(&i.itemid)
                            .map(|s| bucket_series(s, range.from, range.till, SPARK_BUCKETS))
                            .unwrap_or_default(),
                        None => Vec::new(),
                    }
                };
                let mut cpu_s = bucket(&c.cpu_util);
                let mut mem_s = bucket(&mem_item);
                if c.mem_util.is_none() {
                    // pavailable → pused
                    mem_s = crate::metrics::invert_series(&mem_s);
                }
                let _ = &mut cpu_s;
                Some(HostSpark {
                    cpu: cpu_s,
                    mem: mem_s,
                    disk: bucket(&disk_item),
                    disk_mount: disk_max_mount.clone(),
                })
            }
            Err(e) => {
                tracing::debug!("趋势序列获取失败：{e}");
                None
            }
        }
    } else {
        None
    };

    // ---- 附加自定义指标（--keys / 向导选择；支持通配）----

    // ---- 原始逐条数据（--raw）：核心指标的历史采样值 ----
    let raw_samples = if raw {
        // Select core items: CPU util, memory util, disk pused (fullest), and optionally all numeric
        let raw_items: Vec<&ItemRec> = items
            .iter()
            .filter(|i| i.numeric())
            .filter(|i| {
                i.key == "system.cpu.util"
                    || i.key.starts_with("vm.memory.util")
                    || i.key.starts_with("vm.memory.size[pavailable]")
                    || i.key.starts_with("vfs.fs.pused")
                    || i.key.starts_with("vfs.fs.dependent.size[") && i.key.ends_with(",pused]")
                    || i.key.starts_with("vfs.fs.size[") && i.key.ends_with(",pused]")
            })
            .collect();
        if raw_items.is_empty() {
            None
        } else {
            let mut samples = Vec::new();
            for item in &raw_items {
                if let Some(rows) = fetch_history_map(&client, std::slice::from_ref(item), range).await.ok().and_then(|m| m.get(&item.itemid).cloned()) {
                    for s in rows {
                        samples.push(RawSample {
                            key: item.key.clone(),
                            name: item.name.clone(),
                            clock: s.clock,
                            value: s.value,
                            unit: item.units.clone(),
                        });
                    }
                }
            }
            if samples.is_empty() { None } else { Some(samples) }
        }
    } else {
        None
    };

    let extra_map = if !extra_keys.is_empty() {
        let matched: Vec<&ItemRec> = items
            .iter()
            .filter(|i| i.numeric())
            .filter(|i| extra_keys.iter().any(|p| wildcard_match(p, &i.key)))
            .collect();
        if matched.is_empty() {
            None
        } else {
            let stats = aggregate_items(&client, &matched, range).await?;
            Some(
                matched
                    .iter()
                    .map(|i| (i.key.clone(), stats.get(&i.itemid).cloned().unwrap_or_else(|| MetricStats::empty(&i.units))))
                    .collect::<BTreeMap<String, MetricStats>>(),
            )
        }
    } else {
        None
    };

    // ---- 全量指标（--all-items，trend 轻量聚合）----
    let all_map = if all_items {
        let ids: Vec<&ItemRec> = items.iter().filter(|i| i.numeric()).collect();
        let trends = fetch_trend_map(&client, &ids, range).await?;
        let mut m: BTreeMap<String, MetricStats> = BTreeMap::new();
        for it in &ids {
            let st = trends
                .get(&it.itemid)
                .map(|rows| stats_from_trends(it.last_f64(), &it.units, rows))
                .unwrap_or_else(|| MetricStats::empty(&it.units));
            m.insert(it.key.clone(), st);
        }
        Some(m)
    } else {
        None
    };

    let missing_data = cpu.missing && mem.missing && disks.is_empty();
    Ok(HostInspection {
        host: host.clone(),
        os,
        os_family,
        host_disabled: host.status == "1",
        available,
        metrics: MainMetrics { cpu, mem, swap, disk_max, inode_max, uptime_days },
        disks,
        nets,
        services,
        stability: StabilityInfo {
            reboots,
            time_offset_s,
            zombies,
            fd_util,
            cpu_num,
            load1,
            load5,
            load15,
            icmp_loss,
            icmp_latency,
            cert_min_days: cert_days,
            oom_events: None,
        },
        risk: RiskResult::default(),
        problems: Vec::new(),
        all_items: all_map,
        spark,
        extra: extra_map,
        raw_data: raw_samples,
        missing_data,
    })
}

/// 计数器差分（错包/丢包区间新增量，用 history）
async fn counter_delta_of(
    client: &ZabbixClient,
    opt: &Option<ItemRec>,
    range: &TimeRange,
) -> Result<Option<u64>> {
    match opt {
        Some(item) => {
            let hist = fetch_history_map(client, &[item], range).await?;
            Ok(hist.get(&item.itemid).and_then(|s| counter_delta(s)))
        }
        None => Ok(None),
    }
}

/// 计数器型指标改为差分速率统计（重新拉 history 做差分）
async fn rate_stats(client: &ZabbixClient, item: &ItemRec, range: &TimeRange, base: MetricStats) -> MetricStats {
    let hist = match fetch_history_map(client, &[item], range).await {
        Ok(h) => h,
        Err(_) => return base,
    };
    match hist.get(&item.itemid) {
        Some(samples) => {
            let rates = counter_rates(samples);
            match agg_history(&rates) {
                Some((avg, max, min, n)) => MetricStats {
                    cur: rates.last().map(|s| s.value),
                    avg: Some(avg),
                    max: Some(max),
                    min: Some(min),
                    unit: "B/s".into(),
                    count: n,
                    source: "history".into(),
                    missing: false,
                },
                None => base,
            }
        }
        None => base,
    }
}

// ---------- 手动查询 ----------

#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryRow {
    pub host: String,
    pub key: String,
    pub name: String,
    pub stats: MetricStats,
    /// 趋势迷你序列（命中项 ≤30 时生成，供火花线）
    pub trend: Option<Vec<Option<f64>>>,
}

/// 按 key 通配符查询任意监控项统计
pub async fn run_query(
    client: Arc<ZabbixClient>,
    scope: &Scope,
    patterns: &[String],
    range: &TimeRange,
    concurrency: usize,
) -> Result<Vec<QueryRow>> {
    let hosts = resolve_hosts(&client, scope).await?;
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();
    for host in hosts {
        let client = client.clone();
        let patterns = patterns.to_vec();
        let range = range.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            let items = client.get_items(&host.hostid).await?;
            let matched: Vec<&ItemRec> = items
                .iter()
                .filter(|i| i.numeric())
                .filter(|i| patterns.iter().any(|p| wildcard_match(p, &i.key)))
                .collect();
            if matched.is_empty() {
                return Ok(Vec::new());
            }
            let stats = aggregate_items(&client, &matched, &range).await?;
            // 趋势序列（命中项少时才拉，避免大范围查询过重）
            let trend_map: HashMap<String, Vec<Option<f64>>> = if matched.len() <= 30 {
                let refs: Vec<&ItemRec> = matched.clone();
                match fetch_samples(&client, &refs, &range).await {
                    Ok(series) => series
                        .iter()
                        .map(|(k, v)| (k.clone(), bucket_series(v, range.from, range.till, 32)))
                        .collect(),
                    Err(_) => HashMap::new(),
                }
            } else {
                HashMap::new()
            };
            Ok::<_, crate::errors::PatrolError>(
                matched
                    .iter()
                    .filter_map(|i| {
                        stats.get(&i.itemid).map(|s| QueryRow {
                            host: host.host.clone(),
                            key: i.key.clone(),
                            name: i.name.clone(),
                            stats: s.clone(),
                            trend: trend_map.get(&i.itemid).cloned().filter(|v| !v.is_empty()),
                        })
                    })
                    .collect(),
            )
        });
    }
    let mut out = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(r) = joined {
            out.extend(r?);
        }
    }
    out.sort_by(|a, b| a.host.cmp(&b.host).then(a.key.cmp(&b.key)));
    Ok(out)
}

// ---------- 单主机指标趋势图 ----------

pub struct SeriesResult {
    pub title: String,
    pub unit: String,
    pub series: Vec<Option<f64>>,
    pub from: i64,
    pub till: i64,
}

/// 单主机指标趋势序列：metric = cpu | mem | disk（最满分区）；
/// key = Some(任意监控项 key，支持通配，须唯一命中)
pub async fn host_metric_series(
    client: Arc<ZabbixClient>,
    host_name: &str,
    metric: &str,
    range: &TimeRange,
    buckets: usize,
    key: Option<&str>,
) -> Result<SeriesResult> {
    let hosts = client.get_hosts(None, Some(&[host_name.to_string()])).await?;
    let Some(host) = hosts.first() else {
        return Err(crate::errors::PatrolError::Config(format!("未找到主机 {host_name:?}")));
    };
    let items = client.get_items(&host.hostid).await?;
    let rules = CompiledRules::default();
    let c = classify_host(&items, &rules);
    let pick_disk = || -> Option<ItemRec> {
        let mut best: Option<(f64, &ItemRec)> = None;
        for d in &c.disks {
            if let Some(p) = &d.pused {
                let v = p.last_f64().unwrap_or(f64::MIN);
                if best.map(|(bv, _)| v > bv).unwrap_or(true) {
                    best = Some((v, p));
                }
            }
        }
        best.map(|(_, i)| i.clone())
    };
    let (item, title, unit, invert) = if let Some(k) = key {
        // 任意监控项：通配匹配须唯一
        let matched: Vec<&ItemRec> = items
            .iter()
            .filter(|i| i.numeric())
            .filter(|i| wildcard_match(k, &i.key))
            .collect();
        match matched.len() {
            0 => {
                return Err(crate::errors::PatrolError::Config(format!(
                    "{host_name} 没有匹配 {k:?} 的数值监控项"
                )))
            }
            1 => {
                let it = matched[0];
                (Some(it.clone()), format!("{host_name} {}", it.name), it.units.clone(), false)
            }
            n => {
                return Err(crate::errors::PatrolError::Config(format!(
                    "{k:?} 匹配到 {n} 个监控项（如 {:?}…），请写完整 key 或更精确的通配",
                    matched[0].key
                )))
            }
        }
    } else {
        match metric.to_lowercase().as_str() {
            "cpu" => (c.cpu_util.clone(), format!("{host_name} CPU 利用率"), "%".to_string(), false),
            "mem" | "memory" => {
                let invert = c.mem_util.is_none() && c.mem_pavail.is_some();
                (c.mem_util.clone().or_else(|| c.mem_pavail.clone()), format!("{host_name} 内存利用率"), "%".to_string(), invert)
            }
            "disk" => {
                let d = pick_disk();
                let mount = c
                    .disks
                    .iter()
                    .find(|dd| dd.pused.as_ref().map(|p| d.as_ref().map(|x| x.itemid == p.itemid).unwrap_or(false)).unwrap_or(false))
                    .map(|dd| dd.mount.clone())
                    .unwrap_or_default();
                (d, format!("{host_name} 磁盘使用率（最满分区 {mount}）"), "%".to_string(), false)
            }
            other => {
                return Err(crate::errors::PatrolError::Config(format!(
                    "未知指标 {other:?}（支持 cpu | mem | disk，或用 --key 指定任意监控项）"
                )))
            }
        }
    };
    let Some(item) = item else {
        return Err(crate::errors::PatrolError::Config(format!("{host_name} 无 {metric} 监控项")));
    };
    let series = fetch_samples(&client, &[&item], range).await?;
    let samples = series.get(&item.itemid).cloned().unwrap_or_default();
    let mut buckets_v = bucket_series(&samples, range.from, range.till, buckets);
    if invert {
        buckets_v = crate::metrics::invert_series(&buckets_v);
    }
    Ok(SeriesResult { title, unit, series: buckets_v, from: range.from, till: range.till })
}
