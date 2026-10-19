use crate::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportPlan {
    pub source: String,
    pub hosts: Vec<Host>,
    pub warnings: Vec<String>,
}
struct Structure {
    aliases: Vec<String>,
    identities: Vec<Vec<String>>,
    unsupported: Vec<(Vec<String>, String)>,
    visited: HashSet<PathBuf>,
    warnings: Vec<String>,
}
const SUPPORTED: &[&str] = &[
    "hostname",
    "user",
    "port",
    "identityfile",
    "identitiesonly",
    "proxyjump",
    "host",
    "include",
];
fn tokens(line: &str) -> Result<Vec<String>> {
    let mut quote = None;
    let mut escaped = false;
    let mut end = line.len();
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quote != Some('\'') => escaped = true,
            '\'' | '"' if quote == Some(c) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(c),
            '#' if quote.is_none() => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let mut text = line[..end].trim().to_owned();
    if let Some(i) = text.find('=') {
        if !text[..i].contains(char::is_whitespace) {
            text.replace_range(i..=i, " ");
        }
    }
    shell_words::split(&text)
        .map_err(|_| Error::new("invalid_ssh_config", "SSH 配置中有未闭合的引号"))
}
fn read_structure(
    file: &Path,
    s: &mut Structure,
    patterns: &mut Vec<String>,
    stack: &mut HashSet<PathBuf>,
) -> Result<()> {
    let canonical = file.canonicalize().map_err(|e| {
        Error::new(
            "include_unavailable",
            format!("无法读取配置 {}：{e}", file.display()),
        )
    })?;
    // Include a -> b -> a is a real recursive configuration error.
    error::require(
        stack.insert(canonical.clone()),
        "include_cycle",
        "SSH Include 存在循环引用",
    )?;
    s.visited.insert(canonical.clone());
    for (number, line) in std::fs::read_to_string(&canonical)?.lines().enumerate() {
        let words = tokens(line)?;
        if words.is_empty() {
            continue;
        }
        let key = words[0].to_ascii_lowercase();
        let values = &words[1..];
        error::require(
            !values.is_empty(),
            "invalid_ssh_config",
            &format!("{}:{} 缺少选项值", canonical.display(), number + 1),
        )?;
        if key == "match" {
            return Err(Error::new(
                "dynamic_config_requires_manual_import",
                format!(
                    "{}:{} 含 Match 条件，请将所需主机整理为静态配置后导入",
                    canonical.display(),
                    number + 1
                ),
            ));
        }
        if key == "host" {
            *patterns = values.to_vec();
            for v in values {
                if !v.contains(['*', '?', '!']) && !s.aliases.contains(v) {
                    s.aliases.push(v.clone());
                }
            }
        } else if key == "include" {
            for v in values {
                let expanded = expand_home(v)?;
                let path = if expanded.is_absolute() {
                    expanded
                } else {
                    home()?.join(".ssh").join(expanded)
                };
                let files = glob::glob(&path.to_string_lossy())
                    .map_err(|_| Error::new("invalid_include", "Include 路径模式无效"))?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| Error::new("include_unavailable", e.to_string()))?;
                if files.is_empty() {
                    s.warnings
                        .push(format!("Include 未找到文件：{}", path.display()));
                }
                for f in files {
                    read_structure(&f, s, patterns, stack)?;
                }
            }
        } else if key == "identityfile" {
            s.identities.push(patterns.clone());
        } else if !SUPPORTED.contains(&key.as_str()) {
            s.unsupported.push((
                patterns.clone(),
                format!(
                    "{}:{} 未转换选项 {}",
                    canonical.display(),
                    number + 1,
                    words[0]
                ),
            ));
        }
    }
    stack.remove(&canonical);
    Ok(())
}
pub fn home() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| Error::new("home_unavailable", "无法确定用户目录"))
}
pub fn expand_home(value: &str) -> Result<PathBuf> {
    if let Some(v) = value.strip_prefix("~/") {
        Ok(home()?.join(v))
    } else {
        Ok(PathBuf::from(value))
    }
}
fn matches(patterns: &[String], name: &str) -> bool {
    let mut yes = false;
    for p in patterns {
        let (neg, pattern) = p
            .strip_prefix('!')
            .map_or((false, p.as_str()), |s| (true, s));
        let matched = glob::Pattern::new(pattern).is_ok_and(|g| g.matches(name));
        if neg && matched {
            return false;
        }
        if matched {
            yes = true;
        }
    }
    yes
}
pub fn preview(source: &Path, selected: &[String]) -> Result<ImportPlan> {
    let mut structure = Structure {
        aliases: vec![],
        identities: vec![],
        unsupported: vec![],
        visited: HashSet::new(),
        warnings: vec![],
    };
    read_structure(
        source,
        &mut structure,
        &mut vec!["*".into()],
        &mut HashSet::new(),
    )?;
    let mut names = if selected.is_empty() {
        structure.aliases.clone()
    } else {
        selected.to_vec()
    };
    let mut plan = ImportPlan {
        source: source.canonicalize()?.to_string_lossy().into(),
        hosts: vec![],
        warnings: structure.warnings.clone(),
    };
    let mut pending_jumps = BTreeMap::new();
    let mut index = 0;
    while index < names.len() {
        let name = names[index].clone();
        index += 1;
        model::validate_name(&name)?;
        let mut cmd = Command::new("ssh");
        cmd.arg("-G")
            .arg("-F")
            .arg(source)
            .arg("--")
            .arg(&name)
            .env("LC_ALL", "C");
        // Recursive inspection above rejects all Match blocks before OpenSSH can evaluate Match exec.
        let r = process::run(
            cmd,
            None,
            Duration::from_secs(10),
            Arc::new(AtomicBool::new(false)),
            262144,
            None,
        )?;
        error::require(
            r.code == Some(0) && r.termination.is_none() && r.stdout.total <= 262144,
            "ssh_config_resolution_failed",
            &String::from_utf8_lossy(&r.stderr.bytes),
        )?;
        let text = String::from_utf8_lossy(&r.stdout.bytes);
        let mut values = BTreeMap::<String, Vec<String>>::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(' ') {
                values.entry(k.into()).or_default().push(v.into());
            }
        }
        let value = |k: &str| {
            values
                .get(k)
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_default()
        };
        let keys = values
            .get("identityfile")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p != "none")
            .map(|p| expand_home(&p).map(|p| p.to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>>>()?;
        let existing: Vec<_> = keys
            .iter()
            .filter(|p| Path::new(p).is_file())
            .cloned()
            .collect();
        let explicit_keys = structure.identities.iter().any(|p| matches(p, &name));
        let authentication = if existing.is_empty() && !explicit_keys {
            Authentication::Agent
        } else {
            Authentication::Key {
                paths: if existing.is_empty() { keys } else { existing },
                passphrase_ref: None,
            }
        };
        let mut h = Host::new(
            name.clone(),
            value("hostname"),
            value("user"),
            value("port")
                .parse()
                .map_err(|_| Error::new("invalid_port", "SSH 端口无效"))?,
            authentication,
        );
        let c = &mut h.connections[0];
        c.issues = structure
            .unsupported
            .iter()
            .filter(|(p, _)| matches(p, &name))
            .map(|(_, issue)| issue.clone())
            .collect();
        if !structure.warnings.is_empty() {
            c.issues.extend(structure.warnings.clone());
        }
        if let Authentication::Key { paths, .. } = &c.authentication {
            for p in paths {
                if !Path::new(p).is_file() {
                    c.issues.push(format!("密钥文件不存在：{p}"));
                }
            }
        }
        let jump = value("proxyjump");
        if !jump.is_empty() && jump != "none" {
            let jumps: Vec<String> = jump.split(',').map(str::to_owned).collect();
            if jumps.iter().any(|s| model::validate_name(s).is_err()) {
                c.issues
                    .push("跳板带有账号或端口覆盖，请在导入后设置独立登录配置".into());
            } else {
                for j in &jumps {
                    if !names.contains(j) {
                        names.push(j.clone());
                    }
                }
                pending_jumps.insert(name.clone(), jumps);
            }
        }
        h.source = Some(ImportSource {
            path: plan.source.clone(),
            alias: name,
            imported_at: Utc::now(),
            snapshot: serde_json::json!({"address":c.address,"port":c.port,"user":c.user,"identity_files":values.get("identityfile"),"proxy_jump":jump}),
        });
        plan.hosts.push(h);
        error::require(
            plan.hosts.len() <= 256,
            "too_many_hosts",
            "单次导入最多支持 256 个主机",
        )?;
    }
    let ids: BTreeMap<_, _> = plan
        .hosts
        .iter()
        .map(|h| (h.name.clone(), h.default_connection))
        .collect();
    for h in &mut plan.hosts {
        if let Some(j) = pending_jumps.get(&h.name) {
            h.connections[0].jumps = j.iter().map(|name| ids[name]).collect();
        }
    }
    Ok(plan)
}
pub fn apply(store: &Store, plan: ImportPlan) -> Result<usize> {
    store.import_catalog(CatalogExport {
        schema_version: 1,
        hosts: plan.hosts,
    })
}
