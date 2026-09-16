//! 可选业务配置 patrol.toml：自定义指标规则 + 自定义评分规则（存在即覆盖内置评分）。

use crate::rules::{AggKind, MetricRule};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatrolToml {
    pub metrics: Option<MetricsSection>,
    pub scoring: Option<Vec<ScoringRule>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricsSection {
    /// [[metrics.rules]] 列表（见 patrol.example.toml）
    pub rules: Option<Vec<RuleDef>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleDef {
    pub id: String,
    pub pattern: String,
    pub label: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "raw".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScoringRule {
    /// 指标路径：cpu.avg / cpu.max / mem.avg / swap.avg / disk.avg / inode.avg /
    /// load1.avg / offset.max / zombies.cur / fd.cur / cert.cur / net.loss /
    /// bw_util.max / reboots / oom / problems / availability / services
    pub metric: String,
    /// gt | gte | lt | lte | eq
    pub op: String,
    pub value: f64,
    pub points: i64,
    /// 命中即置底 ≥95（直接严重）
    #[serde(default)]
    pub floor: bool,
    /// message 模板，{value} 占位
    #[serde(default)]
    pub message: String,
    /// 分值 × 命中值（用于「每次重启 +15」类规则）
    #[serde(default)]
    pub per_count: bool,
}

impl PatrolToml {
    pub fn load_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| format!("patrol.toml 解析失败：{e}"))
    }

    pub fn metric_rules(&self) -> Vec<MetricRule> {
        self.metrics
            .as_ref()
            .and_then(|m| m.rules.as_ref())
            .map(|rs| {
                rs.iter()
                    .map(|r| MetricRule {
                        id: r.id.clone(),
                        pattern: r.pattern.clone(),
                        label: r.label.clone(),
                        unit: r.unit.clone(),
                        kind: match r.kind.as_str() {
                            "util" => AggKind::Util,
                            "event" => AggKind::Event,
                            _ => AggKind::Raw,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn scoring_rules(&self) -> Option<&[ScoringRule]> {
        self.scoring.as_deref().filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example() {
        let toml_str = r#"
[metrics]
[[metrics.rules]]
id = "jvm_heap"
pattern = '^jvm\.memory\.heap\.pct$'
label = "JVM 堆使用率"
unit = "%"
kind = "util"

[[scoring]]
metric = "cpu.avg"
op = "gte"
value = 85.0
points = 60
message = "CPU 平均 {value}%"
"#;
        let cfg = PatrolToml::load_str(toml_str).unwrap();
        assert_eq!(cfg.metric_rules().len(), 1);
        assert_eq!(cfg.metric_rules()[0].id, "jvm_heap");
        assert!(cfg.scoring_rules().is_some());
        assert_eq!(cfg.scoring_rules().unwrap()[0].value, 85.0);
    }
}
