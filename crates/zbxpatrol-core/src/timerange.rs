//! 时间范围解析：--period / --last / --from+--to 三种写法，优先级从/to > last > period。

use crate::errors::{PatrolError, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub fn seconds(self) -> i64 {
        match self {
            Period::Day => 24 * 3600,
            Period::Week => 7 * 24 * 3600,
            Period::Month => 30 * 24 * 3600,
            Period::Year => 365 * 24 * 3600,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Period::Day => "日",
            Period::Week => "周",
            Period::Month => "月",
            Period::Year => "年",
        }
    }
}

/// 三种时间指定（CLI 已按优先级折叠成一个 TimeSpec）
#[derive(Debug, Clone)]
pub enum TimeSpec {
    Period(Period),
    Last(String),
    FromTo { from: String, to: Option<String> },
}

#[derive(Debug, Clone)]
pub struct TimeRange {
    pub from: i64,
    pub till: i64,
    pub tz: chrono_tz::Tz,
}

impl Default for TimeSpec {
    fn default() -> Self {
        TimeSpec::Period(Period::Day)
    }
}

/// 解析 `48h` / `15d` / `12M` / `30m`（分钟）为秒
pub fn parse_last_duration(s: &str) -> Result<i64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num
        .parse()
        .map_err(|_| PatrolError::Config(format!("--last 格式错误：{s}（示例 48h / 15d / 12M）")))?;
    let secs = match unit {
        "h" | "H" => n * 3600,
        "d" | "D" => n * 86400,
        "M" => n * 30 * 86400,
        "m" => n * 60,
        _ => return Err(PatrolError::Config(format!("--last 单位不支持：{unit}（支持 h/d/M/m）"))),
    };
    if secs <= 0 {
        return Err(PatrolError::Config("--last 时长必须为正".into()));
    }
    Ok(secs)
}

/// 解析 `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM[:SS]`（按指定时区）为 epoch 秒
pub fn parse_datetime(s: &str, tz: chrono_tz::Tz) -> Result<i64> {
    let s = s.trim();
    let naive = if s.len() == 10 {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| PatrolError::Config(format!("日期格式错误 {s:?}：{e}（应为 YYYY-MM-DD）")))?
            .and_hms_opt(0, 0, 0)
            .unwrap()
    } else {
        NaiveDateTime::parse_from_str(&s.replace('T', " "), "%Y-%m-%d %H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(&s.replace('T', " "), "%Y-%m-%d %H:%M"))
            .map_err(|e| {
                PatrolError::Config(format!(
                    "时间格式错误 {s:?}：{e}（应为 \"YYYY-MM-DD[ HH:MM[:SS]]\"）"
                ))
            })?
    };
    tz.from_local_datetime(&naive)
        .single()
        .map(|dt: DateTime<chrono_tz::Tz>| dt.with_timezone(&Utc).timestamp())
        .ok_or_else(|| PatrolError::Config(format!("时间 {s:?} 在时区中不存在或有歧义")))
}

impl TimeRange {
    pub fn resolve(spec: &TimeSpec, tz: chrono_tz::Tz) -> Result<Self> {
        let now = Utc::now().timestamp();
        let (from, till) = match spec {
            TimeSpec::Period(p) => (now - p.seconds(), now),
            TimeSpec::Last(s) => (now - parse_last_duration(s)?, now),
            TimeSpec::FromTo { from, to } => {
                let f = parse_datetime(from, tz)?;
                let t = match to {
                    Some(t) => parse_datetime(t, tz)?,
                    None => now,
                };
                if f >= t {
                    return Err(PatrolError::Config(format!("起始时间必须早于结束时间（{from} >= {t:?}）")));
                }
                (f, t)
            }
        };
        Ok(TimeRange { from, till, tz })
    }

    pub fn seconds(&self) -> i64 {
        self.till - self.from
    }
    pub fn days(&self) -> f64 {
        self.seconds() as f64 / 86400.0
    }

    /// 紧凑区间串：202609010800-202609151759（用于文件名）
    pub fn fmt_compact(&self) -> String {
        format!("{}-{}", fmt_ts(self.from, &self.tz), fmt_ts(self.till, &self.tz))
    }
    /// 人类可读区间串：2026-09-01 08:00 ~ 2026-09-15 17:59
    pub fn fmt_human(&self) -> String {
        format!("{} ~ {}", self.fmt_from(), self.fmt_till())
    }
    pub fn fmt_from(&self) -> String {
        let dt = DateTime::from_timestamp(self.from, 0).unwrap_or_default();
        dt.with_timezone(&self.tz).format("%Y-%m-%d %H:%M").to_string()
    }
    pub fn fmt_till(&self) -> String {
        let dt = DateTime::from_timestamp(self.till, 0).unwrap_or_default();
        dt.with_timezone(&self.tz).format("%Y-%m-%d %H:%M").to_string()
    }
}

fn fmt_ts(ts: i64, tz: &chrono_tz::Tz) -> String {
    let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
    dt.with_timezone(tz).format("%Y%m%d%H%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Asia::Shanghai;

    #[test]
    fn last_duration() {
        assert_eq!(parse_last_duration("48h").unwrap(), 48 * 3600);
        assert_eq!(parse_last_duration("15d").unwrap(), 15 * 86400);
        assert_eq!(parse_last_duration("12M").unwrap(), 12 * 30 * 86400);
        assert_eq!(parse_last_duration("30m").unwrap(), 1800);
        assert!(parse_last_duration("xx").is_err());
    }

    #[test]
    fn datetime_parsing() {
        let ts = parse_datetime("2026-09-01", Shanghai).unwrap();
        // 东八区 2026-09-01 00:00 == UTC 2026-08-31 16:00
        let dt = DateTime::from_timestamp(ts, 0).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M").to_string(), "2026-08-31 16:00");
        let ts2 = parse_datetime("2026-09-01 08:30:00", Shanghai).unwrap();
        assert_eq!(ts2 - ts, 8 * 3600 + 1800);
        assert!(parse_datetime("2026/09/01", Shanghai).is_err());
    }

    #[test]
    fn range_resolve_order() {
        let r = TimeRange::resolve(&TimeSpec::Last("1h".into()), Shanghai).unwrap();
        assert!((r.till - r.from - 3600).abs() < 5);
        let r2 = TimeRange::resolve(
            &TimeSpec::FromTo { from: "2026-09-01".into(), to: Some("2026-09-02".into()) },
            Shanghai,
        )
        .unwrap();
        assert_eq!(r2.till - r2.from, 86400);
        let err = TimeRange::resolve(
            &TimeSpec::FromTo { from: "2026-09-02".into(), to: Some("2026-09-01".into()) },
            Shanghai,
        );
        assert!(err.is_err());
    }
}
