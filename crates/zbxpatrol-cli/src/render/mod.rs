//! 渲染：table（人读）/ json（程序读）/ csv（Excel 直开）；xlsx 单独模块。

pub mod xlsx;
pub mod bilingual;

use crate::Format;
use crate::lang::t;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use zbxpatrol_core::pipeline::QueryRow;
use zbxpatrol_core::timerange::TimeRange;
use zbxpatrol_core::types::{fmt_opt, GroupRec, MetricStats, ReportData};

fn mk_table() -> Table {
    let mut t = Table::new();
    t.load_preset(ASCII_MARKDOWN);
    t
}

pub fn groups(groups: &[GroupRec], fmt: Format) {
    match fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(groups).unwrap()),
        Format::Csv => {
            println!("\u{FEFF}groupid,name");
            for g in groups {
                println!("{},{}", g.groupid, csv_escape(&g.name));
            }
        }
        _ => {
            let mut tbl = mk_table();
            tbl.set_header(vec![t("Group ID", "群组ID"), t("Group Name", "群组名称")]);
            for g in groups {
                tbl.add_row(vec![&g.groupid, &g.name]);
            }
            println!("{tbl}");
        }
    }
}


/// 主机列表（hosts 子命令；含系统类型与群组）
pub fn hosts_list(hosts: &[zbxpatrol_core::types::HostInfo], fmt: Format) {
    match fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(hosts).unwrap()),
        Format::Csv => {
            println!("\u{FEFF}host,name,ip,os,groups");
            for h in hosts {
                println!(
                    "{},{},{},{},{}",
                    csv_escape(&h.host),
                    csv_escape(&h.name),
                    h.ip,
                    csv_escape(&h.os_family),
                    csv_escape(&h.groups.join("|"))
                );
            }
        }
        _ => {
            let mut tbl = mk_table();
            tbl.set_header(vec![t("Host", "主机"), t("Visible Name", "可见名"), t("IP", "IP"), t("OS", "系统"), t("Groups", "群组")]);
            for h in hosts {
                tbl.add_row(vec![
                    &h.host,
                    &h.name,
                    &h.ip,
                    &h.os_family,
                    &h.groups.join("|"),
                ]);
            }
            println!("{tbl}");
        }
    }
}

// ---------- 趋势图 ----------

const SPARK_BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Unicode 火花线：▁▂▃▅▆█（无数据段为空格）
pub fn sparkline(series: &[Option<f64>]) -> String {
    let vals: Vec<f64> = series.iter().filter_map(|v| *v).collect();
    if vals.is_empty() {
        return "—".into();
    }
    let min = vals.iter().cloned().fold(f64::MAX, f64::min);
    let max = vals.iter().cloned().fold(f64::MIN, f64::max);
    let span = (max - min).max(1e-9);
    series
        .iter()
        .map(|v| match v {
            Some(v) => {
                let idx = ((v - min) / span * 7.0).round() as usize;
                SPARK_BLOCKS[idx.min(7)]
            }
            None => ' ',
        })
        .collect()
}

/// 全尺寸 ASCII 趋势图（控制台输出）
pub fn ascii_chart(title: &str, unit: &str, series: &[Option<f64>], from_label: &str, till_label: &str) {
    const H: usize = 14;
    let vals: Vec<f64> = series.iter().filter_map(|v| *v).collect();
    if vals.is_empty() || series.is_empty() {
        println!("{title}: {}", t("no data in range", "区间内无数据"));
        return;
    }
    let min = vals.iter().cloned().fold(f64::MAX, f64::min);
    let max = vals.iter().cloned().fold(f64::MIN, f64::max);
    let span = (max - min).max(1e-9);
    let avg = vals.iter().sum::<f64>() / vals.len() as f64;
    println!("{title}  [{unit}]  {}: {min:.1} / {}: {avg:.1} / {}: {max:.1}", t("min","最小"), t("avg","平均"), t("max","最大"));
    let lvl = |v: f64| -> usize { (((v - min) / span * (H - 1) as f64).round() as usize).min(H - 1) };
    let avg_lvl = lvl(avg);
    for row in (0..H).rev() {
        let y = min + span * row as f64 / (H - 1) as f64;
        let mut line = String::from(" │");
        for v in series {
            let ch = match v {
                Some(v) if lvl(*v) >= row => '█',
                Some(_) => {
                    if row == avg_lvl {
                        '┄'
                    } else {
                        ' '
                    }
                }
                None => {
                    if row == avg_lvl {
                        '┄'
                    } else {
                        ' '
                    }
                }
            };
            line.push(ch);
        }
        println!("{y:>6.1}{line}");
    }
    let w = series.len();
    let mut axis = String::from(" └");
    axis.push_str(&"─".repeat(w));
    println!("{axis}");
    println!("       {from_label:<width$}{till_label:>width$}", width = w.saturating_sub(from_label.chars().count()),);
}

pub fn items_aggregated(rows: &[(String, String, String, String, usize)], fmt: Format) {
    match fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(rows).unwrap()),
        Format::Csv => {
            println!("\u{FEFF}key,name,unit,value_type,hosts");
            for (k, n, u, v, c) in rows {
                println!("{},{},{},{},{}", csv_escape(k), csv_escape(n), u, v, c);
            }
        }
        _ => {
            let mut tbl = mk_table();
            tbl.set_header(vec!["key", t("Name","名称"), t("Unit","单位"), t("Type","类型"), t("Hosts Covered","覆盖主机数")]);
            for (k, n, u, v, c) in rows {
                tbl.add_row(vec![k, n, u, v, &c.to_string()]);
            }
            println!("{tbl}");
        }
    }
}

pub fn items_detail(rows: &[(String, String, String, String, String)], fmt: Format) {
    match fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(rows).unwrap()),
        Format::Csv => {
            println!("\u{FEFF}host,key,name,lastvalue,units");
            for r in rows {
                println!("{},{},{},{},{}", csv_escape(&r.0), csv_escape(&r.1), csv_escape(&r.2), csv_escape(&r.3), csv_escape(&r.4));
            }
        }
        _ => {
            let mut tbl = mk_table();
            tbl.set_header(vec![t("Host","主机"), "key", t("Name","名称"), t("Last Value","当前值"), t("Unit","单位")]);
            for r in rows {
                tbl.add_row(vec![&r.0, &r.1, &r.2, &r.3, &r.4]);
            }
            println!("{tbl}");
        }
    }
}

pub fn query(rows: &[QueryRow], range: &TimeRange, fmt: Format) {
    match fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(rows).unwrap()),
        _ => {
            eprintln!("{}: {}", t("Query range","查询区间"), range.fmt_human());
            let mut tbl = mk_table();
            tbl.set_header(vec![t("Host","主机"), "key", t("Cur","当前"), t("Avg","平均"), t("Max","最大"), t("Min","最小"), t("Trend","趋势"), t("Unit","单位"), t("Source","来源")]);
            for r in rows {
                let spark = r.trend.as_ref().map(|s| sparkline(s)).unwrap_or_else(|| "—".into());
                tbl.add_row(vec![
                    &r.host,
                    &r.key,
                    &fmt_opt(r.stats.cur),
                    &fmt_opt(r.stats.avg),
                    &fmt_opt(r.stats.max),
                    &fmt_opt(r.stats.min),
                    &spark,
                    &r.stats.unit,
                    &r.stats.source,
                ]);
            }
            println!("{tbl}");
        }
    }
}

pub fn query_csv_stdout(rows: &[QueryRow]) {
    print!("\u{FEFF}");
    for (i, r) in rows.iter().enumerate() {
        if i == 0 {
            println!("host,key,cur,avg,max,min,unit,source");
        }
        println!(
            "{},{},{},{},{},{},{},{}",
            r.host,
            csv_escape(&r.key),
            opt_csv(r.stats.cur),
            opt_csv(r.stats.avg),
            opt_csv(r.stats.max),
            opt_csv(r.stats.min),
            csv_escape(&r.stats.unit),
            r.stats.source
        );
    }
}

pub fn query_csv_file(rows: &[QueryRow], path: &std::path::Path) -> std::io::Result<()> {
    let mut s = String::from("\u{FEFF}host,key,cur,avg,max,min,unit,source\n");
    for r in rows {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            r.host,
            csv_escape(&r.key),
            opt_csv(r.stats.cur),
            opt_csv(r.stats.avg),
            opt_csv(r.stats.max),
            opt_csv(r.stats.min),
            r.stats.unit,
            r.stats.source,
        ));
    }
    std::fs::write(path, s)
}

fn opt_csv(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}")).unwrap_or_default()
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 报表摘要（默认 table 模式 stdout 输出）
pub fn report_summary(data: &ReportData, xlsx_path: Option<&std::path::Path>, data_json: Option<&std::path::Path>) {
    let s = &data.summary;
    let zh = crate::lang::is_zh();
    let strict_label = if zh { data.strictness.clone() } else { bilingual::strictness(&data.strictness) };
    println!("========== {} ==========", t("Inspection Overview", "巡检总览"));
    println!("{} : {}", t("Range", "巡检区间"), data.range.human);
    println!("{} : {}", t("Scope", "范围"), scope_label(data));
    println!("{} : {}", t("Scoring", "评分模式"), strict_label);
    if zh {
        println!("主机     : {} 台（可用 {} / 不可达 {} / 数据缺失 {}）", s.host_total, s.available, s.unavailable, s.missing_data);
        println!("风险分布 : 健康 {} | 低危 {} | 中危 {} | 高危 {} | 严重 {}", s.risk_dist.healthy, s.risk_dist.low, s.risk_dist.medium, s.risk_dist.high, s.risk_dist.critical);
        println!("未恢复问题: {} 个（区间内新增 {}，区间前遗留 {}）", s.problem_open, s.problem_new_in_range, s.problem_carried_over);
    } else {
        println!("Hosts     : {} (up {} / down {} / missing {})", s.host_total, s.available, s.unavailable, s.missing_data);
        println!("Risk dist : Healthy {} | Low {} | Medium {} | High {} | Critical {}", s.risk_dist.healthy, s.risk_dist.low, s.risk_dist.medium, s.risk_dist.high, s.risk_dist.critical);
        println!("Open problems: {} (new in range {}, carried over {})", s.problem_open, s.problem_new_in_range, s.problem_carried_over);
    }
    if !s.top_risk.is_empty() && s.top_risk[0].score > 0 {
        println!("{}:", t("Top Risk Hosts", "TOP 风险主机"));
        let mut tbl = mk_table();
        tbl.set_header(vec![t("Host", "主机"), t("Score", "风险分"), t("Level", "等级")]);
        for tr in &s.top_risk {
            if tr.score > 0 {
                let level = if zh { tr.level.clone() } else { bilingual::risk_level(&tr.level) };
                tbl.add_row(vec![&tr.host, &tr.score.to_string(), &level]);
            }
        }
        println!("{tbl}");
    } else {
        println!("{}: {}", t("TOP 风险主机", "Top Risk"), t("None (all healthy)", "无（全部健康）"));
    }
    if let Some(x) = xlsx_path {
        println!("{} : {}", t("Report file", "报表文件"), x.display());
    }
    if let Some(dj) = data_json {
        println!("{}: {}", t("JSON data", "JSON 数据"), dj.display());
    }
}

/// 控制台主机明细表（彩色，按风险分降序）
pub fn report_console_detail(data: &ReportData) {
    use comfy_table::Color;
    let zh = crate::lang::is_zh();
    let mut tbl = mk_table();
    if zh {
        tbl.set_header(vec![
            "主机", "IP", "系统", "可用", "CPU 当/均/峰/谷%", "CPU趋势", "内存 当/均/峰/谷%", "内存趋势",
            "磁盘最满", "磁盘 当/均/峰/谷%", "Swap均%", "负载1", "风险分", "等级", "主要风险点",
        ]);
    } else {
        tbl.set_header(vec![
            "Host", "IP", "OS", "Status", "CPU cur/avg/max/min%", "CPU trend", "Mem cur/avg/max/min%", "Mem trend",
            "Disk (fullest)", "Disk cur/avg/max/min%", "Swap avg%", "Load1", "Score", "Level", "Risk points",
        ]);
    }
    let pct_color = |v: f64| -> Color {
        if v >= 90.0 {
            Color::Red
        } else if v >= 75.0 {
            Color::Yellow
        } else {
            Color::Green
        }
    };
    let quad = |s: &MetricStats| -> comfy_table::Cell {
        let color = pct_color(s.avg.or(s.cur).unwrap_or(0.0));
        let txt = format!("{}/{}/{}/{}", fmt_opt(s.cur), fmt_opt(s.avg), fmt_opt(s.max), fmt_opt(s.min));
        comfy_table::Cell::new(txt).fg(color)
    };
    let mut hosts: Vec<&zbxpatrol_core::types::HostInspection> = data.hosts.iter().collect();
    hosts.sort_by_key(|h| std::cmp::Reverse(h.risk.score));
    for h in hosts {
        let mut row: Vec<comfy_table::Cell> = Vec::new();
        row.push(comfy_table::Cell::new(&h.host.host));
        row.push(comfy_table::Cell::new(&h.host.ip));
        row.push(comfy_table::Cell::new(if h.os_family.is_empty() { "—" } else { &h.os_family }));
        row.push(if h.available {
            comfy_table::Cell::new(t("Up", "正常")).fg(Color::Green)
        } else {
            comfy_table::Cell::new(t("Down", "不可达")).fg(Color::Red)
        });
        row.push(quad(&h.metrics.cpu));
        row.push(comfy_table::Cell::new(
            h.spark.as_ref().map(|s| sparkline(&s.cpu)).unwrap_or_else(|| "—".into()),
        ));
        row.push(quad(&h.metrics.mem));
        row.push(comfy_table::Cell::new(
            h.spark.as_ref().map(|s| sparkline(&s.mem)).unwrap_or_else(|| "—".into()),
        ));
        match &h.metrics.disk_max {
            Some(d) => {
                row.push(comfy_table::Cell::new(&d.mount));
                row.push(quad(&d.space));
            }
            None => {
                row.push(comfy_table::Cell::new("—"));
                row.push(comfy_table::Cell::new("—"));
            }
        }
        row.push(match h.metrics.swap.as_ref().and_then(|s| s.avg) {
            Some(v) => comfy_table::Cell::new(format!("{v:.1}")).fg(pct_color(v)),
            None => comfy_table::Cell::new("—"),
        });
        row.push(comfy_table::Cell::new(
            h.stability.load1.as_ref().and_then(|l| l.avg).map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into()),
        ));
        row.push(comfy_table::Cell::new(h.risk.score.to_string()));
        let level_display = if zh { h.risk.level.clone() } else { bilingual::risk_level(&h.risk.level) };
        row.push(comfy_table::Cell::new(&level_display).fg(match h.risk.level.as_str() {
            "严重" => Color::Red,
            "高危" => Color::Yellow,
            "中危" => Color::Cyan,
            "低危" => Color::Blue,
            _ => Color::Green,
        }));
        let points = if zh { h.risk.points.join("；") } else { bilingual::risk_point(&h.risk.points.join("; ")) };
        row.push(comfy_table::Cell::new(points));
        tbl.add_row(row);
    }
    println!("========== {} ==========", t("Host Details (by risk)", "主机明细（按风险降序）"));
    println!("{tbl}");
}

fn scope_label(data: &ReportData) -> String {
    match data.scope_type.as_str() {
        "all" => t("All hosts", "全部主机").to_string(),
        "group" => format!("{} {}", t("Group", "群组"), data.scope_names.join("、")),
        _ => data.scope_names.join("、"),
    }
}

/// 主机明细平面 CSV（stdout）
pub fn report_csv_stdout(data: &ReportData) {
    let out = report_csv_content(data);
    println!("{out}");
}

/// 主机明细平面 CSV（写文件，UTF-8 BOM，Excel 直开）
pub fn report_csv_file(data: &ReportData, path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, report_csv_content(data))
}

fn report_csv_content(data: &ReportData) -> String {
    report_csv_string(data)
}

/// 主机明细 CSV 内容（不含 BOM；serve 流式/落盘复用）
pub fn report_csv_string(data: &ReportData) -> String {
    // CSV 表头固定英文（供程序/Excel 消费，语言无关）
    let zh = crate::lang::is_zh();
    let mut out = String::from("\u{FEFF}");
    out.push_str("Host,IP,Group,Status,Score,Level,CPU cur%,CPU avg%,CPU max%,CPU min%,Mem cur%,Mem avg%,Mem max%,Mem min%,Disk (fullest),Disk cur%,Disk avg%,Disk max%,Disk min%,Risk Points\n");
    for h in &data.hosts {
        let disk = h.metrics.disk_max.as_ref();
        let level = if zh { h.risk.level.clone() } else { bilingual::risk_level(&h.risk.level) };
        let avail = if zh { if h.available { "正常" } else { "不可达" } } else { if h.available { "OK" } else { "Unreachable" } };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            h.host.host,
            h.host.ip,
            csv_escape(&h.host.groups.join("|")),
            avail,
            h.risk.score,
            level,
            fmt_opt(h.metrics.cpu.cur),
            fmt_opt(h.metrics.cpu.avg),
            fmt_opt(h.metrics.cpu.max),
            fmt_opt(h.metrics.cpu.min),
            fmt_opt(h.metrics.mem.cur),
            fmt_opt(h.metrics.mem.avg),
            fmt_opt(h.metrics.mem.max),
            fmt_opt(h.metrics.mem.min),
            disk.map(|d| d.mount.clone()).unwrap_or_default(),
            disk.map(|d| fmt_opt(d.space.cur)).unwrap_or_default(),
            disk.map(|d| fmt_opt(d.space.avg)).unwrap_or_default(),
            disk.map(|d| fmt_opt(d.space.max)).unwrap_or_default(),
            disk.map(|d| fmt_opt(d.space.min)).unwrap_or_default(),
            csv_escape(&h.risk.points.join("；")),
        ));
    }
    out
}
