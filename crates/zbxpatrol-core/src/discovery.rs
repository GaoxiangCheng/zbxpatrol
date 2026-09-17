//! 发现：范围解析（all/group/hosts）+ 主机 item 分类（规则引擎归位）。

use crate::errors::{PatrolError, Result};
use crate::rules::CompiledRules;
use crate::types::{HostInfo, ItemRec, Scope};
use crate::zabbix::ZabbixClient;

/// 把 CLI 的范围参数解析为目标主机列表（群组名自动映射 groupid，未知主机报错）
pub async fn resolve_hosts(client: &ZabbixClient, scope: &Scope) -> Result<Vec<HostInfo>> {
    let hosts = match scope {
        Scope::All => client.get_hosts(None, None).await?,
        Scope::Groups(names) => {
            let groups = client.get_groups().await?;
            let mut ids = Vec::new();
            let mut missing = Vec::new();
            for n in names {
                match groups.iter().find(|g| &g.name == n) {
                    Some(g) => ids.push(g.groupid.clone()),
                    None => missing.push(n.clone()),
                }
            }
            if !missing.is_empty() {
                return Err(PatrolError::Config(format!(
                    "未找到主机群组：{}（可用 zbxpatrol groups 查看）",
                    missing.join("、")
                )));
            }
            client.get_hosts(Some(&ids), None).await?
        }
        Scope::Hosts(names) => {
            let hosts = client.get_hosts(None, Some(names)).await?;
            let found: Vec<&str> = hosts.iter().map(|h| h.host.as_str()).collect();
            let missing: Vec<&String> =
                names.iter().filter(|n| !found.contains(&n.as_str())).collect();
            if !missing.is_empty() {
                return Err(PatrolError::Config(format!(
                    "未找到主机：{}（可用 zbxpatrol hosts 查看；注意大小写需完全一致）",
                    missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("、")
                )));
            }
            hosts
        }
    };
    if hosts.is_empty() {
        return Err(PatrolError::Config("范围内没有主机（群组为空？）".into()));
    }
    Ok(hosts)
}

// ---------- 每台主机的分类结果 ----------

#[derive(Debug, Clone, Default)]
pub struct DiskItems {
    pub mount: String,
    pub pused: Option<ItemRec>,
    pub used: Option<ItemRec>,
    pub total: Option<ItemRec>,
    pub inode_pused: Option<ItemRec>,
    pub inode_pfree: Option<ItemRec>,
}

#[derive(Debug, Clone, Default)]
pub struct NetItems {
    pub ifname: String,
    pub in_bytes: Option<ItemRec>,
    pub out_bytes: Option<ItemRec>,
    pub in_err: Option<ItemRec>,
    pub out_err: Option<ItemRec>,
    pub in_drop: Option<ItemRec>,
    pub out_drop: Option<ItemRec>,
    pub speed: Option<ItemRec>,
}

#[derive(Debug, Clone, Default)]
pub struct HostClassified {
    pub cpu_util: Option<ItemRec>,
    pub mem_util: Option<ItemRec>,
    pub mem_pavail: Option<ItemRec>,
    pub mem_total: Option<ItemRec>,
    pub swap_util: Option<ItemRec>,
    pub swap_pfree: Option<ItemRec>,
    pub load1: Option<ItemRec>,
    pub load5: Option<ItemRec>,
    pub load15: Option<ItemRec>,
    pub cpu_num: Option<ItemRec>,
    pub uptime: Option<ItemRec>,
    pub boottime: Option<ItemRec>,
    pub zombies: Option<ItemRec>,
    pub proc_num: Option<ItemRec>,
    pub localtime: Option<ItemRec>,
    pub agent_avail: Option<ItemRec>,
    pub maxfiles: Option<ItemRec>,
    pub openfiles: Option<ItemRec>,
    pub os_uname: Option<ItemRec>,
    pub icmp_loss: Option<ItemRec>,
    pub icmp_sec: Option<ItemRec>,
    pub cert_days: Option<ItemRec>,
    pub disks: Vec<DiskItems>,
    pub nets: Vec<NetItems>,
    pub services: Vec<ItemRec>,
    pub io_awaits: Vec<ItemRec>,
    /// 其余全部数值型 item（--all-items 用）
    pub others_numeric: Vec<ItemRec>,
}

/// 从 system.uname / system.sw.os 推导系统类型：返回 (类别, 详情)
pub fn os_family_of(items: &[ItemRec]) -> (String, String) {
    let detail = ["system.uname", "system.sw.os", "system.sw.os[short]"]
        .iter()
        .find_map(|k| {
            items
                .iter()
                .find(|i| &i.key == k)
                .and_then(|i| i.lastvalue.clone())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_default();
    let d = detail.to_lowercase();
    let family = if d.contains("windows") || d.contains("microsoft") {
        "Windows"
    } else if d.contains("linux") {
        "Linux"
    } else if d.contains("freebsd") {
        "FreeBSD"
    } else if d.contains("darwin") || d.contains("mac os") {
        "macOS"
    } else if d.contains("aix") {
        "AIX"
    } else if d.contains("hp-ux") {
        "HP-UX"
    } else if d.contains("sunos") || d.contains("solaris") {
        "Solaris"
    } else if detail.is_empty() {
        "未知"
    } else {
        "其他"
    };
    (family.to_string(), detail)
}

fn disk_by_mount<'a>(list: &'a mut Vec<DiskItems>, mount: &str) -> &'a mut DiskItems {
    if let Some(i) = list.iter().position(|d| d.mount == mount) {
        &mut list[i]
    } else {
        list.push(DiskItems { mount: mount.into(), ..Default::default() });
        list.last_mut().unwrap()
    }
}

fn net_by_if<'a>(list: &'a mut Vec<NetItems>, ifname: &str) -> &'a mut NetItems {
    if let Some(i) = list.iter().position(|n| n.ifname == ifname) {
        &mut list[i]
    } else {
        list.push(NetItems { ifname: ifname.into(), ..Default::default() });
        list.last_mut().unwrap()
    }
}

/// 对一台主机的全部 item 做规则归类
pub fn classify_host(items: &[ItemRec], rules: &CompiledRules) -> HostClassified {
    let mut c = HostClassified::default();
    for it in items {
        // OS 信息取自字符型 uname
        if it.key == "system.uname" {
            c.os_uname = Some(it.clone());
            continue;
        }
        if !it.numeric() {
            continue;
        }
        let Some(rule) = rules.match_key(&it.key) else {
            c.others_numeric.push(it.clone());
            continue;
        };
        match rule.id.as_str() {
            "cpu_util" => c.cpu_util = Some(it.clone()),
            "mem_util" => c.mem_util = Some(it.clone()),
            "mem_pavail" => c.mem_pavail = Some(it.clone()),
            "mem_total" => c.mem_total = Some(it.clone()),
            "swap_util" => c.swap_util = Some(it.clone()),
            "swap_pfree" => c.swap_pfree = Some(it.clone()),
            "load1" => c.load1 = Some(it.clone()),
            "load5" => c.load5 = Some(it.clone()),
            "load15" => c.load15 = Some(it.clone()),
            "cpu_num" => c.cpu_num = Some(it.clone()),
            "uptime" => c.uptime = Some(it.clone()),
            "boottime" => c.boottime = Some(it.clone()),
            "zombies" => c.zombies = Some(it.clone()),
            "proc_num" => c.proc_num = Some(it.clone()),
            "localtime" => c.localtime = Some(it.clone()),
            "agent_avail" => c.agent_avail = Some(it.clone()),
            "maxfiles" => c.maxfiles = Some(it.clone()),
            "openfiles" => c.openfiles = Some(it.clone()),
            "icmp_loss" => c.icmp_loss = Some(it.clone()),
            "icmp_sec" => c.icmp_sec = Some(it.clone()),
            "cert_days" => c.cert_days = Some(it.clone()),
            "disk_pused" | "disk_pused_old" => {
                if let Some(m) = crate::rules::mount_of(&it.key) {
                    disk_by_mount(&mut c.disks, &m).pused = Some(it.clone());
                }
            }
            "disk_used" => {
                if let Some(m) = crate::rules::mount_of(&it.key) {
                    disk_by_mount(&mut c.disks, &m).used = Some(it.clone());
                }
            }
            "disk_total" => {
                if let Some(m) = crate::rules::mount_of(&it.key) {
                    disk_by_mount(&mut c.disks, &m).total = Some(it.clone());
                }
            }
            "inode_pused" => {
                if let Some(m) = crate::rules::mount_of(&it.key) {
                    disk_by_mount(&mut c.disks, &m).inode_pused = Some(it.clone());
                }
            }
            "inode_pfree" => {
                if let Some(m) = crate::rules::mount_of(&it.key) {
                    disk_by_mount(&mut c.disks, &m).inode_pfree = Some(it.clone());
                }
            }
            "if_in" | "if_in_bytes" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).in_bytes = Some(it.clone());
                }
            }
            "if_out" | "if_out_bytes" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).out_bytes = Some(it.clone());
                }
            }
            "if_in_err" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).in_err = Some(it.clone());
                }
            }
            "if_out_err" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).out_err = Some(it.clone());
                }
            }
            "if_in_drop" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).in_drop = Some(it.clone());
                }
            }
            "if_out_drop" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).out_drop = Some(it.clone());
                }
            }
            "if_speed" => {
                if let Some(n) = crate::rules::ifname_of(&it.key) {
                    net_by_if(&mut c.nets, &n).speed = Some(it.clone());
                }
            }
            "tcp_service" => c.services.push(it.clone()),
            "io_await" => c.io_awaits.push(it.clone()),
            _ => c.others_numeric.push(it.clone()),
        }
    }
    // 剔除没有任何指标的空壳
    c.disks.retain(|d| d.pused.is_some() || d.total.is_some() || d.used.is_some());
    c.nets.retain(|n| n.in_bytes.is_some() || n.out_bytes.is_some() || n.speed.is_some());
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, vt: u8) -> ItemRec {
        ItemRec {
            itemid: key.into(),
            hostid: "1".into(),
            key: key.into(),
            name: key.into(),
            value_type: vt,
            units: String::new(),
            lastvalue: None,
            lastclock: None,
            status: "0".into(),
        }
    }

    #[test]
    fn classify_real_env_items() {
        let items = vec![
            item("system.cpu.util", 0),
            item("vm.memory.utilization", 0),
            item("vm.memory.size[pavailable]", 0),
            item("system.swap.size[,pfree]", 0),
            item("system.boottime", 3),
            item("vfs.fs.dependent.size[/,pused]", 0),
            item("vfs.fs.dependent.size[/,total]", 3),
            item("vfs.fs.dependent.inode[/,pfree]", 0),
            item("net.if.in[\"ens3\"]", 3),
            item("net.if.out[\"ens3\"]", 3),
            item("net.if.in[\"ens3\",errors]", 3),
            item("net.tcp.port[<1.2.3.4>,9092]", 3),
            item("system.uname", 1),
            item("bigdata.flink.state", 3),
            item("agent.version", 1),
        ];
        let c = classify_host(&items, &CompiledRules::default());
        assert!(c.cpu_util.is_some());
        assert!(c.mem_util.is_some());
        assert!(c.swap_pfree.is_some());
        assert_eq!(c.disks.len(), 1);
        assert_eq!(c.disks[0].mount, "/");
        assert!(c.disks[0].inode_pfree.is_some());
        assert_eq!(c.nets.len(), 1);
        assert_eq!(c.nets[0].ifname, "ens3");
        assert_eq!(c.services.len(), 1);
        assert!(c.os_uname.is_some());
        // 未匹配的数值项进入 others；字符型不进
        assert_eq!(c.others_numeric.len(), 1);
        assert_eq!(c.others_numeric[0].key, "bigdata.flink.state");
    }
}
