//! Excel 报表渲染（rust_xlsxwriter）：总览/主机明细/磁盘/网络服务/稳定性/问题/评分说明/全指标。

use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook};
use zbxpatrol_core::types::{fmt_opt, MetricStats, ReportData};
use crate::lang::{t, is_zh};
use super::bilingual;

const GREEN: u32 = 0xC6EFCE;
const YELLOW: u32 = 0xFFEB9C;
const RED: u32 = 0xFFC7CE;
const HEADER_BG: u32 = 0x2F5496;
const TITLE_BG: u32 = 0xD9E2F3;

fn header_fmt() -> Format {
    Format::new()
        .set_bold()
        .set_font_color(Color::RGB(0xFFFFFF))
        .set_background_color(Color::RGB(HEADER_BG))
        .set_align(FormatAlign::Center)
        .set_text_wrap()
}

fn title_fmt() -> Format {
    Format::new().set_bold().set_font_size(14).set_background_color(Color::RGB(TITLE_BG)).set_align(FormatAlign::Center)
}

fn bold() -> Format {
    Format::new().set_bold()
}

fn num_fmt() -> Format {
    Format::new().set_num_format("0.0")
}

/// 按使用率取色：<75 绿 / 75-89 黄 / ≥90 红
fn pct_fmt(v: Option<f64>) -> Format {
    let bg = match v {
        Some(x) if x >= 90.0 => RED,
        Some(x) if x >= 75.0 => YELLOW,
        Some(_) => GREEN,
        None => 0xFFFFFF,
    };
    Format::new().set_num_format("0.0").set_background_color(Color::RGB(bg))
}

fn risk_fmt(level: &str) -> Format {
    let bg = match level {
        "严重" => RED,
        "高危" => 0xFFD966,
        "中危" => YELLOW,
        "低危" => 0xE2EFDA,
        _ => GREEN,
    };
    Format::new().set_background_color(Color::RGB(bg)).set_align(FormatAlign::Center)
}

fn fmt_gb(bytes: Option<f64>) -> String {
    bytes.map(|b| format!("{:.1}", b / 1024.0 / 1024.0 / 1024.0)).unwrap_or_else(|| "—".into())
}

fn ts_str(ts: i64, tz: &str) -> String {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::Asia::Shanghai);
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.with_timezone(&tz).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

fn write_row(sheet: &mut rust_xlsxwriter::Worksheet, row: u32, cells: &[String]) -> Result<(), rust_xlsxwriter::XlsxError> {
    for (c, v) in cells.iter().enumerate() {
        sheet.write(row, c as u16, v.as_str())?;
    }
    Ok(())
}

pub fn render_report(data: &ReportData, path: &std::path::Path) -> Result<(), rust_xlsxwriter::XlsxError> {
    let mut wb = Workbook::new();

    sheet_overview(&mut wb, data)?;
    sheet_hosts(&mut wb, data)?;
    sheet_disks(&mut wb, data)?;
    sheet_nets(&mut wb, data)?;
    sheet_stability(&mut wb, data)?;
    sheet_problems(&mut wb, data)?;
    sheet_scoring_doc(&mut wb, data)?;
    if data.hosts.iter().any(|h| h.extra.is_some()) {
        sheet_extra_items(&mut wb, data)?;
    }
    if data.hosts.iter().any(|h| h.all_items.is_some()) {
        sheet_all_items(&mut wb, data)?;
    }
    if data.hosts.iter().any(|h| h.raw_data.is_some()) {
        sheet_raw_data(&mut wb, data)?;
    }
    wb.save(path)?;
    Ok(())
}

/// 渲染到内存缓冲（HTTP API ?format=xlsx 用）
pub fn render_report_buffer(data: &ReportData) -> Result<Vec<u8>, rust_xlsxwriter::XlsxError> {
    let mut wb = Workbook::new();
    sheet_overview(&mut wb, data)?;
    sheet_hosts(&mut wb, data)?;
    sheet_disks(&mut wb, data)?;
    sheet_nets(&mut wb, data)?;
    sheet_stability(&mut wb, data)?;
    sheet_problems(&mut wb, data)?;
    sheet_scoring_doc(&mut wb, data)?;
    if data.hosts.iter().any(|h| h.extra.is_some()) {
        sheet_extra_items(&mut wb, data)?;
    }
    if data.hosts.iter().any(|h| h.all_items.is_some()) {
        sheet_all_items(&mut wb, data)?;
    }
    if data.hosts.iter().any(|h| h.raw_data.is_some()) {
        sheet_raw_data(&mut wb, data)?;
    }
    wb.save_to_buffer()
}

// ---------- 原始数据明细（--raw） ----------

fn sheet_raw_data(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Raw Data", "原始数据"))?;
    let headers = ["Host", "Key", "Name", "Timestamp", "Value", "Unit"];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    let tz: chrono_tz::Tz = data.range.tz.parse().unwrap_or(chrono_tz::Asia::Shanghai);
    let mut r = 1u32;
    for host in &data.hosts {
        if let Some(samples) = &host.raw_data {
            for sample in samples {
                s.write(r, 0, host.host.host.as_str())?;
                s.write(r, 1, sample.key.as_str())?;
                s.write(r, 2, sample.name.as_str())?;
                let dt = chrono::DateTime::from_timestamp(sample.clock, 0)
                    .map(|d| d.with_timezone(&tz).format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                s.write(r, 3, dt.as_str())?;
                s.write(r, 4, sample.value)?;
                s.write(r, 5, sample.unit.as_str())?;
                r += 1;
            }
        }
    }
    Ok(())
}

// ---------- 自定义指标明细（--keys 附加项） ----------

fn sheet_extra_items(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Custom Items", "自定义指标明细"))?;
    let headers = [t("Host","主机"), "key", t("Cur","当前"), t("Avg","平均"), t("Max","最大"), t("Min","最小"), t("Unit","单位"), t("Source","来源")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    let mut r = 1u32;
    for h in &data.hosts {
        if let Some(extra) = &h.extra {
            for (key, st) in extra {
                s.write(r, 0, h.host.host.as_str())?;
                s.write(r, 1, key.as_str())?;
                s.write(r, 2, fmt_opt(st.cur).as_str())?;
                s.write(r, 3, fmt_opt(st.avg).as_str())?;
                s.write(r, 4, fmt_opt(st.max).as_str())?;
                s.write(r, 5, fmt_opt(st.min).as_str())?;
                s.write(r, 6, st.unit.as_str())?;
                s.write(r, 7, st.source.as_str())?;
                r += 1;
            }
        }
    }
    Ok(())
}

// ---------- Sheet 1: 总览 ----------

fn sheet_overview(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Overview", "巡检总览"))?;
    s.set_column_width(0, 22)?;
    s.set_column_width(1, 60)?;
    s.merge_range(0, 0, 0, 1, t("Zabbix Inspection Report", "Zabbix 服务器巡检报告"), &title_fmt())?;
    let mut r = 2u32;
    let zh = is_zh();
    let kv = |k: &str, v: String| -> (String, String) { (k.to_string(), v) };
    let scope_txt = if zh {
        match data.scope_type.as_str() {
            "all" => "全部主机".to_string(),
            "group" => format!("主机群组：{}", data.scope_names.join("、")),
            _ => format!("主机：{}", data.scope_names.join("、")),
        }
    } else {
        match data.scope_type.as_str() {
            "all" => "All hosts".to_string(),
            "group" => format!("Group: {}", data.scope_names.join(", ")),
            _ => format!("Hosts: {}", data.scope_names.join(", ")),
        }
    };
    let strict_label = if zh { data.strictness.clone() } else { bilingual::strictness(&data.strictness) };
    let rows = if zh {
        vec![
            kv("巡检区间", data.range.human.clone()),
            kv("报表生成时间", ts_str(data.generated_at, &data.range.tz)),
            kv("巡检范围", scope_txt),
            kv("评分模式", strict_label),
            kv("主机总数", format!("{} 台", data.summary.host_total)),
            kv("可用 / 不可达", format!("{} / {} 台", data.summary.available, data.summary.unavailable)),
            kv("数据缺失主机", format!("{} 台", data.summary.missing_data)),
            kv("风险分布", format!("健康 {}｜低危 {}｜中危 {}｜高危 {}｜严重 {}",
                data.summary.risk_dist.healthy, data.summary.risk_dist.low, data.summary.risk_dist.medium,
                data.summary.risk_dist.high, data.summary.risk_dist.critical)),
            kv("未恢复问题", format!("{} 个（区间内新增 {}，区间前遗留 {}）",
                data.summary.problem_open, data.summary.problem_new_in_range, data.summary.problem_carried_over)),
            kv("停用主机", format!("{}", data.summary.host_disabled)),
        ]
    } else {
        vec![
            kv("Range", data.range.human.clone()),
            kv("Generated at", ts_str(data.generated_at, &data.range.tz)),
            kv("Scope", scope_txt),
            kv("Scoring", strict_label),
            kv("Total hosts", format!("{}", data.summary.host_total)),
            kv("Up / Down", format!("{} / {}", data.summary.available, data.summary.unavailable)),
            kv("Missing data", format!("{}", data.summary.missing_data)),
            kv("Risk distribution", format!("Healthy {} | Low {} | Medium {} | High {} | Critical {}",
                data.summary.risk_dist.healthy, data.summary.risk_dist.low, data.summary.risk_dist.medium,
                data.summary.risk_dist.high, data.summary.risk_dist.critical)),
            kv("Open problems", format!("{} (new in range {}, carried over {})",
                data.summary.problem_open, data.summary.problem_new_in_range, data.summary.problem_carried_over)),
            kv("Disabled hosts", format!("{}", data.summary.host_disabled)),
        ]
    };
    for (k, v) in rows {
        s.write(r, 0, k.as_str())?.write(r, 1, v.as_str())?;
        r += 1;
    }
    // TOP 风险
    r += 1;
    s.write_with_format(r, 0, t("TOP Risk Hosts", "TOP 风险主机"), &bold())?;
    r += 1;
    write_row(s, r, &[t("Rank","排名").to_string(), t("Host / Score / Level","主机 / 风险分 / 等级").to_string()])?;
    for (i, tr) in data.summary.top_risk.iter().enumerate() {
        if tr.score == 0 {
            break;
        }
        write_row(s, r + 1 + i as u32, &[format!("{}", i + 1), format!("{}（{} 分，{}）", tr.host, tr.score, tr.level)])?;
    }
    // 结论建议
    let mut r2 = r + 1 + data.summary.top_risk.len() as u32 + 1;
    s.write_with_format(r2, 0, t("Conclusion & Advice", "结论与建议"), &bold())?;
    r2 += 1;
    let risky: Vec<&zbxpatrol_core::types::HostInspection> =
        data.hosts.iter().filter(|h| h.risk.score >= 75).collect();
    if risky.is_empty() {
        write_row(s, r2, &["—".into(), "本轮巡检范围内主机整体健康，无高危及以上风险。".into()])?;
    } else {
        for (i, h) in risky.iter().enumerate() {
            let advice = h.risk.points.first().cloned().unwrap_or_default();
            write_row(s, r2 + i as u32, &[format!("{}", i + 1), format!("{}（{} 分，{}）：{}", h.host.host, h.risk.score, h.risk.level, advice)])?;
        }
    }
    let footer = r2 + risky.len() as u32 + 2;
    s.write(footer, 0, format!("生成：zbxpatrol v{}", data.version).as_str())?;
    Ok(())
}

// ---------- Sheet 2: 主机明细 ----------

fn sheet_hosts(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Host Details", "主机明细"))?;
    let headers: Vec<&str> = if is_zh() {
        vec![
            "主机名", "可见名", "IP", "群组", "OS", "可用性", "运行天数",
            "CPU当前%", "CPU平均%", "CPU最大%", "CPU最小%",
            "内存当前%", "内存平均%", "内存最大%", "内存最小%",
            "磁盘最满分区", "磁盘当前%", "磁盘平均%", "磁盘最大%", "磁盘最小%",
            "Swap平均%", "inode最满分区", "inode平均%",
            "负载1(平均)", "风险分", "风险等级", "主要风险点",
        ]
    } else {
        vec![
            "Host", "Visible Name", "IP", "Group", "OS", "Status", "Uptime (days)",
            "CPU cur%", "CPU avg%", "CPU max%", "CPU min%",
            "Mem cur%", "Mem avg%", "Mem max%", "Mem min%",
            "Disk (fullest)", "Disk cur%", "Disk avg%", "Disk max%", "Disk min%",
            "Swap avg%", "Inode (fullest)", "Inode avg%",
            "Load1 (avg)", "Score", "Level", "Risk Points",
        ]
    };
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    s.set_column_width(26, 80)?;
    for c in 0..6u16 {
        s.set_column_width(c, 18)?;
    }
    for row_i in 0..data.hosts.len() {
        let h = &data.hosts[row_i];
        let r = row_i as u32 + 1;
        let mut c = 0u16;
        s.write(r, c, h.host.host.as_str())?; c += 1;
        s.write(r, c, h.host.name.as_str())?; c += 1;
        s.write(r, c, h.host.ip.as_str())?; c += 1;
        s.write(r, c, h.host.groups.join(",").as_str())?; c += 1;
        s.write(r, c, h.os.as_str())?; c += 1;
        let avail = if h.host_disabled { "停用".to_string() } else if h.available { "正常".to_string() } else { "不可达".to_string() };
        s.write_with_format(r, c, avail, &if h.available { pct_fmt(None) } else { pct_fmt(Some(99.0)) })?; c += 1;
        s.write_with_format(r, c, h.metrics.uptime_days.unwrap_or(-1.0), &num_fmt())?; c += 1;
        // CPU / 内存
        for st in [&h.metrics.cpu, &h.metrics.mem] {
            for v in [st.cur, st.avg, st.max, st.min] {
                write_pct(s, r, c, v)?; c += 1;
            }
        }
        // 磁盘最满
        match &h.metrics.disk_max {
            Some(d) => {
                s.write(r, c, d.mount.as_str())?; c += 1;
                for v in [d.space.cur, d.space.avg, d.space.max, d.space.min] {
                    write_pct(s, r, c, v)?; c += 1;
                }
            }
            None => {
                for _ in 0..5 { s.write(r, c, "—")?; c += 1; }
            }
        }
        // swap avg
        match &h.metrics.swap {
            Some(sw) => write_pct(s, r, c, sw.avg.or(sw.cur))?,
            None => { s.write(r, c, "—")?; }
        }
        c += 1;
        // inode 最满
        match &h.metrics.inode_max {
            Some(d) => {
                s.write(r, c, d.mount.as_str())?; c += 1;
                write_pct(s, r, c, d.space.avg.or(d.space.cur))?;
            }
            None => {
                s.write(r, c, "—")?; c += 1;
                s.write(r, c, "—")?;
            }
        }
        c += 1;
        // 负载
        if let Some(l1) = &h.stability.load1 {
            s.write_with_format(r, c, l1.avg.unwrap_or(0.0), &num_fmt())?;
        } else {
            s.write(r, c, "—")?;
        }
        c += 1;
        // 风险
        s.write_with_format(r, c, h.risk.score, &Format::new().set_align(FormatAlign::Center))?; c += 1;
        let level_display = if is_zh() { h.risk.level.clone() } else { bilingual::risk_level(&h.risk.level) };
        s.write_with_format(r, c, level_display.as_str(), &risk_fmt(&h.risk.level))?; c += 1;
        s.write(r, c, h.risk.points.join("；\n").as_str())?;
    }
    Ok(())
}

fn write_pct(s: &mut rust_xlsxwriter::Worksheet, r: u32, c: u16, v: Option<f64>) -> Result<(), rust_xlsxwriter::XlsxError> {
    match v {
        Some(x) => {
            s.write_with_format(r, c, x, &pct_fmt(Some(x)))?;
        }
        None => {
            s.write_with_format(r, c, "—", &pct_fmt(None))?;
        }
    }
    Ok(())
}

// ---------- Sheet 3: 磁盘分区 ----------

fn sheet_disks(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Disk Partitions", "磁盘分区明细"))?;
    let headers = [t("Host","主机"), t("Mount","挂载点"), t("Total(GB)","总量(GB)"), t("Used(GB)","已用(GB)"), t("Cur%","当前%"), t("Avg%","平均%"), t("Max%","最大%"), t("Min%","最小%"), t("inode Cur%","inode当前%"), t("inode Avg%","inode平均%"), t("Days to Full","预计满盘(天)")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    let mut r = 1u32;
    for h in &data.hosts {
        for d in &h.disks {
            s.write(r, 0, h.host.host.as_str())?;
            s.write(r, 1, d.mount.as_str())?;
            s.write(r, 2, fmt_gb(d.total_b).as_str())?;
            s.write(r, 3, fmt_gb(d.used_b).as_str())?;
            for (c, v) in [d.space.cur, d.space.avg, d.space.max, d.space.min].iter().enumerate() {
                write_pct(s, r, 4 + c as u16, *v)?;
            }
            match &d.inode {
                Some(i) => {
                    write_pct(s, r, 8, i.cur)?;
                    write_pct(s, r, 9, i.avg)?;
                }
                None => {
                    s.write(r, 8, "—")?;
                    s.write(r, 9, "—")?;
                }
            }
            match d.forecast_days {
                Some(dd) => {
                    s.write(r, 10, format!("{dd:.0}").as_str())?;
                }
                None => {
                    s.write(r, 10, "—")?;
                }
            }
            r += 1;
        }
    }
    Ok(())
}

// ---------- Sheet 4: 网络与服务 ----------

fn sheet_nets(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Network & Services", "网络与服务"))?;
    let headers = [t("Host","主机"), t("NIC","网卡"), t("In (Mbps) cur/avg/max","入带宽(Mbps)当前/平均/最大"), t("Out (Mbps) cur/avg/max","出带宽(Mbps)当前/平均/最大"), t("Peak Util%","带宽峰值利用率%"), t("Errors + (in/out)","错包新增(入/出)"), t("Drops + (in/out)","丢包新增(入/出)"), t("ICMP loss avg%","ICMP丢包avg%"), t("ICMP rtt avg(ms)","ICMP延迟avg(ms)")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    let mut r = 1u32;
    for h in &data.hosts {
        for n in &h.nets {
            s.write(r, 0, h.host.host.as_str())?;
            s.write(r, 1, n.ifname.as_str())?;
            s.write(r, 2, tri(n.in_mbps.as_ref()))?;
            s.write(r, 3, tri(n.out_mbps.as_ref()))?;
            write_pct(s, r, 4, n.util_pct.as_ref().and_then(|u| u.max))?;
            let errs = format!("{}/{}", n.in_errors.unwrap_or(0), n.out_errors.unwrap_or(0));
            s.write(r, 5, errs.as_str())?;
            let drops = format!("{}/{}", n.in_dropped.unwrap_or(0), n.out_dropped.unwrap_or(0));
            s.write(r, 6, drops.as_str())?;
            if let Some(l) = &h.stability.icmp_loss {
                write_pct(s, r, 7, l.avg)?;
            }
            if let Some(sec) = &h.stability.icmp_latency {
                s.write(r, 8, format!("{:.1}", sec.avg.unwrap_or(0.0) * 1000.0).as_str())?;
            }
            r += 1;
        }
        // 服务探测
        for svc in &h.services {
            s.write(r, 0, h.host.host.as_str())?;
            s.write(r, 1, "服务探测")?;
            s.write(r, 2, svc.name.as_str())?;
            s.write(r, 3, svc.key.as_str())?;
            s.write_with_format(r, 4, if svc.ok { "正常" } else { "失败" }, &if svc.ok { pct_fmt(None) } else { pct_fmt(Some(99.0)) })?;
            r += 1;
        }
    }
    Ok(())
}

fn tri(st: Option<&MetricStats>) -> String {
    match st {
        Some(s) => format!("{}/{}/{}", fmt_opt(s.cur), fmt_opt(s.avg), fmt_opt(s.max)),
        None => "—".into(),
    }
}

// ---------- Sheet 5: 稳定性与安全 ----------

fn sheet_stability(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Stability & Security", "稳定性与安全"))?;
    let headers = [t("Host","主机"), t("Reboots in range","区间重启次数"), t("Clock offset max(s)","时间偏移max(s)"), t("Zombies","僵尸进程"), t("fd util%","fd使用率%"), t("CPU cores","CPU核数"), t("Load1 avg","1分钟负载avg"), t("Cert min days","证书最小天数")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    for (i, h) in data.hosts.iter().enumerate() {
        let r = i as u32 + 1;
        s.write(r, 0, h.host.host.as_str())?;
        s.write(r, 1, h.stability.reboots.map(|v| v.to_string()).unwrap_or_else(|| "—".into()).as_str())?;
        if let Some(off) = &h.stability.time_offset_s {
            s.write_with_format(r, 2, off.max.unwrap_or(0.0), &num_fmt())?;
        } else {
            s.write(r, 2, "—")?;
        }
        if let Some(z) = &h.stability.zombies {
            s.write_with_format(r, 3, z.cur.unwrap_or(0.0), &num_fmt())?;
        } else {
            s.write(r, 3, "—")?;
        }
        if let Some(f) = &h.stability.fd_util {
            write_pct(s, r, 4, f.cur)?;
        } else {
            s.write(r, 4, "—")?;
        }
        s.write(r, 5, h.stability.cpu_num.map(|v| v.to_string()).unwrap_or_else(|| "—".into()).as_str())?;
        if let Some(l) = &h.stability.load1 {
            s.write_with_format(r, 6, l.avg.unwrap_or(0.0), &num_fmt())?;
        } else {
            s.write(r, 6, "—")?;
        }
        if let Some(c) = &h.stability.cert_min_days {
            s.write_with_format(r, 7, c.cur.unwrap_or(-1.0), &num_fmt())?;
        } else {
            s.write(r, 7, "—")?;
        }
    }
    Ok(())
}

// ---------- Sheet 6: 问题与告警 ----------

fn sheet_problems(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Problems & Alerts", "问题与告警"))?;
    let headers = [t("Time","时间"), t("Host","主机"), t("Severity","级别"), t("Description","内容"), t("Status","状态"), t("Acknowledged","已确认")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    s.set_column_width(3, 80)?;
    for (i, p) in data.problems.iter().enumerate() {
        let r = i as u32 + 1;
        s.write(r, 0, ts_str(p.clock, &data.range.tz).as_str())?;
        s.write(r, 1, p.hosts.join(",").as_str())?;
        let sev = Format::new().set_background_color(Color::RGB(if p.severity >= 4 { RED } else if p.severity >= 3 { YELLOW } else { 0xFFFFFF }));
        s.write_with_format(r, 2, p.severity_label.as_str(), &sev)?;
        s.write(r, 3, p.name.as_str())?;
        s.write(r, 4, if p.disabled { "停用" } else if p.recovered { "已恢复" } else { "未恢复" })?;
        s.write(r, 5, if p.acknowledged { "是" } else { "否" })?;
    }
    Ok(())
}

// ---------- Sheet 7: 评分说明 ----------

fn sheet_scoring_doc(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("Scoring Guide", "评分说明"))?;
    let mode = if data.strictness.starts_with("宽松") {
        zbxpatrol_core::scoring::Strictness::Loose
    } else if data.strictness.starts_with("严格") {
        zbxpatrol_core::scoring::Strictness::Strict
    } else {
        zbxpatrol_core::scoring::Strictness::Standard
    };
    let fmt_levels = |l: [f64; 4]| format!("{}/{}/{}/{}", l[0] as i64, l[1] as i64, l[2] as i64, l[3] as i64);
    let rows: [[&str; 3]; 15] = [
        ["维度", "规则", "分值"],
        ["评分模式", &format!("{}（基准 {}%，宽松90/标准80/严格70，可用 --strictness 切换）", mode.label(), mode.base() as i64), "—"],
        ["可用性", "主机不可达 / 服务探测失败", "直接严重（≥95）"],
        ["CPU", &format!("平均 {} 阶梯（严重/高危/中危/低危）", fmt_levels(mode.util_levels())), "60/45/30/15"],
        ["CPU", &format!("峰值 ≥{}%", mode.cpu_peak() as i64), "+10"],
        ["CPU", "1分钟负载 > 核数×1.5", "+10"],
        ["内存", &format!("平均 {} 阶梯", fmt_levels(mode.mem_levels())), "60/45/30/15"],
        ["内存", &format!("Swap 平均 ≥{}%", mode.swap_thr() as i64), "+10"],
        ["磁盘", &format!("空间或 inode {} 阶梯", fmt_levels(mode.disk_levels())), "60/45/30/15"],
        ["磁盘", "按当前增速预计 90 天内满盘", "+15"],
        ["网络", &format!("ICMP 丢包>0 / 带宽峰值≥{}% / 错包新增 / fd≥{}%", mode.pct_thr() as i64, mode.pct_thr() as i64), "+10/+10/+5/+10"],
        ["稳定性", "重启 +15/次；OOM +10；僵尸>10 +5；时间偏移>30s +10；证书<30天 +15", "见左"],
        ["告警", "未恢复高危/灾难问题每个 +5", "+5"],
        ["等级", "0-39 健康 / 40-59 低危 / 60-74 中危 / 75-89 高危 / 90-100 严重", "—"],
        ["自定义", "patrol.toml [[scoring]] 规则（存在即替换内置，不受模式影响）", "自定义"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if r == 0 {
                s.write_with_format(r as u32, c as u16, *v, &header_fmt())?;
            } else {
                s.write(r as u32, c as u16, *v)?;
            }
        }
    }
    s.set_column_width(1, 70)?;
    Ok(())
}

// ---------- Sheet 8: 全部指标明细（--all-items） ----------

fn sheet_all_items(wb: &mut Workbook, data: &ReportData) -> Result<(), rust_xlsxwriter::XlsxError> {
    let s = wb.add_worksheet();
    s.set_name(t("All Items", "全部指标明细"))?;
    let headers = [t("Host","主机"), "key", t("Cur","当前"), t("Avg","平均"), t("Max","最大"), t("Min","最小"), t("Unit","单位"), t("Source","来源")];
    for (c, h) in headers.iter().enumerate() {
        s.write_with_format(0, c as u16, *h, &header_fmt())?;
    }
    s.set_freeze_panes(1, 0)?;
    let mut r = 1u32;
    'outer: for h in &data.hosts {
        if let Some(items) = &h.all_items {
            for (key, st) in items {
                if r > 50000 {
                    break 'outer;
                }
                s.write(r, 0, h.host.host.as_str())?;
                s.write(r, 1, key.as_str())?;
                s.write(r, 2, fmt_opt(st.cur).as_str())?;
                s.write(r, 3, fmt_opt(st.avg).as_str())?;
                s.write(r, 4, fmt_opt(st.max).as_str())?;
                s.write(r, 5, fmt_opt(st.min).as_str())?;
                s.write(r, 6, st.unit.as_str())?;
                s.write(r, 7, st.source.as_str())?;
                r += 1;
            }
        }
    }
    Ok(())
}
