//! 问题/告警：严重级别中文映射与汇总。

use crate::types::ProblemRec;

pub fn severity_label(sev: u8) -> &'static str {
    match sev {
        0 => "未分类",
        1 => "信息",
        2 => "警告",
        3 => "一般",
        4 => "高危",
        5 => "灾难",
        _ => "未知",
    }
}

/// 区间内未恢复的高危/灾难问题数（评分用）
pub fn open_high_severity(problems: &[ProblemRec]) -> usize {
    problems
        .iter()
        .filter(|p| !p.recovered && p.severity >= 4)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(severity_label(4), "高危");
        assert_eq!(severity_label(5), "灾难");
    }
}
