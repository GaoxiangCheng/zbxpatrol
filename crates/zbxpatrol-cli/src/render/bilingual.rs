//! Bilingual mapping tables for report content.
//! Core produces Chinese; this module translates to English when --lang en.

/// Translate a risk level string (健康/低危/中危/高危/严重 → English)
pub fn risk_level(s: &str) -> String {
    let en = match s {
        "健康" => "Healthy",
        "低危" => "Low",
        "中危" => "Medium",
        "高危" => "High",
        "严重" => "Critical",
        _ => s,
    };
    en.to_string()
}

/// Translate severity label (未分类/信息/警告/一般/高危/灾难 → English)
#[allow(dead_code)]
pub fn severity(s: &str) -> String {
    let en = match s {
        "未分类" => "Not Classified",
        "信息" => "Information",
        "警告" => "Warning",
        "一般" => "Average",
        "高危" => "High",
        "灾难" => "Disaster",
        _ => s,
    };
    en.to_string()
}

/// Translate strictness label (标准（基准 80%）→ Standard (80% baseline))
pub fn strictness(s: &str) -> String {
    let en = match s {
        x if x.starts_with("宽松") => "Loose (90% baseline)",
        x if x.starts_with("标准") => "Standard (80% baseline)",
        x if x.starts_with("严格") => "Strict (70% baseline)",
        _ => s,
    };
    en.to_string()
}

/// Translate availability
#[allow(dead_code)]
pub fn availability(ok: bool) -> String {
    if ok { "OK".into() } else { "Unreachable".into() }
}

/// Translate "正常"/"不可达" in existing strings
#[allow(dead_code)]
pub fn availability_str(s: &str) -> String {
    match s {
        "正常" => "OK".into(),
        "不可达" => "Unreachable".into(),
        _ => s.to_string(),
    }
}

/// Common risk point keywords for partial translation
pub fn risk_point(s: &str) -> String {
    if !crate::lang::is_zh() {
        // Simple keyword replacement for English output
        s.replace("使用率", " usage ")
            .replace("达到严重阈值", "reached CRITICAL threshold")
            .replace("达到高危阈值", "reached HIGH threshold")
            .replace("达到中危阈值", "reached MEDIUM threshold")
            .replace("达到低危阈值", "reached LOW threshold")
            .replace("平均使用率", "avg usage")
            .replace("CPU 峰值", "CPU peak")
            .replace("内存", "Memory")
            .replace("磁盘最满分区", "Disk (fullest partition)")
            .replace("主机不可达", "Host unreachable")
            .replace("服务探测失败", "Service check failed")
            .replace("区间内发生", "occurred in range: ")
            .replace("次重启", " reboots")
            .replace("个未恢复的高危/灾难告警", " unresolved high/disaster alerts")
    } else {
        s.to_string()
    }
}
