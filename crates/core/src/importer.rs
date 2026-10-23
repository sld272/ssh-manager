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
    /// Options that managed connections replace with their own settings; they never block a host.
    pub ignored: Vec<String>,
    /// Match blocks are never evaluated, so their settings are not imported.
    pub skipped: Vec<String>,
}
struct SkippedMatch {
    location: String,
    criteria: String,
    host_patterns: Vec<String>,
    original_patterns: Vec<String>,
    options: Vec<String>,
}
enum Block {
    Host(Vec<String>),
    Match,
}
struct Structure {
    aliases: Vec<String>,
    identities: Vec<Vec<String>>,
    unsupported: Vec<(Vec<String>, String)>,
    ignored: Vec<(Vec<String>, String)>,
    skipped: Vec<SkippedMatch>,
    warnings: Vec<String>,
    /// Copies without Match blocks; `ssh -G` reads these so Match exec can never run.
    sanitized: tempfile::TempDir,
    copies: usize,
}
const SUPPORTED: &[&str] = &[
    "hostname",
    "user",
    "port",
    "identityfile",
    "identitiesonly",
    "identityagent",
    "proxyjump",
    "preferredauthentications",
    "pubkeyauthentication",
    "passwordauthentication",
    "host",
    "include",
];
const MANAGED: &str = "连接参数由 ssh-manager 统一设置";
const TRUST: &str = "主机信任由 ssh-manager 单独保存";
const FORWARDING: &str = "ssh-manager 不转发 agent、图形界面或端口";
const PASSPHRASE: &str = "密钥口令通过凭据绑定或 ssh-agent 提供";
const IGNORED: &[(&str, &str)] = &[
    ("ignoreunknown", MANAGED),
    ("serveraliveinterval", MANAGED),
    ("serveralivecountmax", MANAGED),
    ("tcpkeepalive", MANAGED),
    ("connecttimeout", MANAGED),
    ("connectionattempts", MANAGED),
    ("controlmaster", MANAGED),
    ("controlpath", MANAGED),
    ("controlpersist", MANAGED),
    ("compression", MANAGED),
    ("loglevel", MANAGED),
    ("batchmode", MANAGED),
    ("requesttty", MANAGED),
    ("numberofpasswordprompts", MANAGED),
    ("kbdinteractiveauthentication", MANAGED),
    ("userknownhostsfile", TRUST),
    ("globalknownhostsfile", TRUST),
    ("stricthostkeychecking", TRUST),
    ("updatehostkeys", TRUST),
    ("checkhostip", TRUST),
    ("hashknownhosts", TRUST),
    ("visualhostkey", TRUST),
    ("forwardagent", FORWARDING),
    ("forwardx11", FORWARDING),
    ("forwardx11trusted", FORWARDING),
    ("localforward", FORWARDING),
    ("remoteforward", FORWARDING),
    ("dynamicforward", FORWARDING),
    ("exitonforwardfailure", FORWARDING),
    ("usekeychain", PASSPHRASE),
    ("addkeystoagent", PASSPHRASE),
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
    block: &mut Block,
    stack: &mut HashSet<PathBuf>,
) -> Result<PathBuf> {
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
    s.copies += 1;
    let copy = s.sanitized.path().join(format!("{}.conf", s.copies));
    let mut out = String::new();
    for (number, line) in std::fs::read_to_string(&canonical)?.lines().enumerate() {
        let words = tokens(line)?;
        if words.is_empty() {
            continue;
        }
        let key = words[0].to_ascii_lowercase();
        let values = &words[1..];
        let location = format!("{}:{}", canonical.display(), number + 1);
        error::require(
            !values.is_empty(),
            "invalid_ssh_config",
            &format!("{location} 缺少选项值"),
        )?;
        if key == "match" {
            if values.len() == 1 && values[0].eq_ignore_ascii_case("all") {
                // "Match all" ends a Host block so the following lines apply to every host, exactly like "Host *".
                *block = Block::Host(vec!["*".into()]);
                out.push_str("Host *\n");
            } else {
                s.skipped.push(skipped_match(location, values));
                *block = Block::Match;
            }
            continue;
        }
        // A Match block, including one that ends an included file, lasts until the next Host line.
        if matches!(block, Block::Match) && key != "host" {
            if let Some(m) = s.skipped.last_mut()
                && !m.options.contains(&words[0])
            {
                m.options.push(words[0].clone());
            }
            continue;
        }
        if key == "host" {
            *block = Block::Host(values.to_vec());
            for v in values {
                if !v.contains(['*', '?', '!']) && !s.aliases.contains(v) {
                    s.aliases.push(v.clone());
                }
            }
        }
        let Block::Host(patterns) = &*block else {
            unreachable!("Match lines continue above")
        };
        let patterns = patterns.clone();
        if key == "include" {
            let mut copies = vec![];
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
                    copies.push(read_structure(&f, s, block, stack)?);
                }
            }
            if !copies.is_empty() {
                let paths: Vec<_> = copies
                    .iter()
                    .map(|p| format!("\"{}\"", p.display()))
                    .collect();
                out.push_str(&format!("Include {}\n", paths.join(" ")));
            }
            continue;
        }
        if key == "identityfile" {
            s.identities.push(patterns);
        } else if let Some((_, reason)) = IGNORED.iter().find(|(k, _)| *k == key) {
            s.ignored.push((
                patterns,
                format!("{location} 未导入 {}：{reason}", words[0]),
            ));
        } else if !SUPPORTED.contains(&key.as_str()) {
            s.unsupported
                .push((patterns, format!("{location} 未转换选项 {}", words[0])));
        }
        out.push_str(line);
        out.push('\n');
    }
    stack.remove(&canonical);
    std::fs::write(&copy, out)?;
    Ok(copy)
}
fn skipped_match(location: String, values: &[String]) -> SkippedMatch {
    let mut m = SkippedMatch {
        location,
        criteria: values.join(" "),
        host_patterns: vec![],
        original_patterns: vec![],
        options: vec![],
    };
    let mut words = values.iter();
    while let Some(word) = words.next() {
        let criterion = word.to_ascii_lowercase();
        if matches!(criterion.as_str(), "all" | "canonical" | "final") {
            continue;
        }
        // "Match host" without a pattern list is malformed; OpenSSH rejects it, and it names no host here.
        let Some(argument) = words.next() else { break };
        let patterns = argument.split(',').map(str::to_owned);
        // A negated criterion such as "!host web" does not say which hosts the block applies to.
        match criterion.as_str() {
            "host" => m.host_patterns.extend(patterns),
            "originalhost" => m.original_patterns.extend(patterns),
            _ => {}
        }
    }
    m
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
        ignored: vec![],
        skipped: vec![],
        warnings: vec![],
        sanitized: tempfile::Builder::new().prefix("sshm-import-").tempdir()?,
        copies: 0,
    };
    let sanitized = read_structure(
        source,
        &mut structure,
        &mut Block::Host(vec!["*".into()]),
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
        ignored: vec![],
        skipped: structure
            .skipped
            .iter()
            .map(|m| {
                if m.options.is_empty() {
                    format!("{} 跳过 Match {}", m.location, m.criteria)
                } else {
                    format!(
                        "{} 跳过 Match {}（其中的 {} 未导入）",
                        m.location,
                        m.criteria,
                        m.options.join("、")
                    )
                }
            })
            .collect(),
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
            .arg(&sanitized)
            .arg("--")
            .arg(&name)
            .env("LC_ALL", "C");
        // The sanitized copies contain no Match blocks, so OpenSSH cannot evaluate Match exec here.
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
        // OpenSSH 9.9 prints these as true/false; earlier releases print yes/no.
        let enabled = |k: &str| !matches!(value(k).as_str(), "no" | "false");
        let preferred = value("preferredauthentications");
        let method = match preferred.as_str() {
            "" => "publickey,password",
            v => v,
        }
        .split(',')
        .find(|m| {
            (*m == "publickey" && enabled("pubkeyauthentication"))
                || (*m == "password" && enabled("passwordauthentication"))
        })
        .map(str::to_owned);
        let authentication = if method.as_deref() == Some("password") {
            Authentication::Password {
                credential_ref: None,
            }
        } else if keys.is_empty() || (existing.is_empty() && !explicit_keys) {
            Authentication::Agent
        } else {
            Authentication::Key {
                paths: if explicit_keys { keys } else { existing },
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
        c.identities_only = value("identitiesonly") == "yes";
        c.issues = structure
            .unsupported
            .iter()
            .filter(|(p, _)| matches(p, &name))
            .map(|(_, issue)| issue.clone())
            .collect();
        if !structure.warnings.is_empty() {
            c.issues.extend(structure.warnings.clone());
        }
        let hostname = value("hostname");
        for m in &structure.skipped {
            if matches(&m.host_patterns, &hostname) || matches(&m.original_patterns, &name) {
                c.issues.push(format!(
                    "{} 的 Match 条件可能适用于此主机，其中的设置未导入，请确认后补充",
                    m.location
                ));
            }
        }
        // "PreferredAuthentications keyboard-interactive" leaves neither key nor password authentication.
        if method.is_none() {
            c.issues.push(
                "SSH 配置只允许 ssh-manager 不支持的认证方式，请导入后选择密钥、ssh-agent 或密码"
                    .into(),
            );
        }
        let agent = value("identityagent");
        if !agent.is_empty() && !matches!(c.authentication, Authentication::Password { .. }) {
            if agent.starts_with('/') {
                c.identity_agent = Some(agent.clone());
            } else {
                // "IdentityAgent SSH_AUTH_SOCK" or "none" names no socket that a managed connection can use.
                c.issues.push(format!(
                    "IdentityAgent {agent} 不是套接字路径，请导入后指定 ssh-agent 套接字"
                ));
            }
        }
        if let Authentication::Key { paths, .. } = &c.authentication {
            for p in paths {
                if !Path::new(p).is_file() {
                    c.issues.push(format!("密钥文件不存在：{p}"));
                }
            }
        }
        if c.identities_only && matches!(c.authentication, Authentication::Agent) {
            c.issues
                .push("仅使用指定身份，但没有可用的密钥文件，请重新选择认证方式".into());
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
            snapshot: serde_json::json!({"address":c.address,"port":c.port,"user":c.user,"identity_files":values.get("identityfile"),"proxy_jump":jump,"identity_agent":agent}),
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
    plan.ignored = structure
        .ignored
        .iter()
        .filter(|(p, _)| plan.hosts.iter().any(|h| matches(p, &h.name)))
        .map(|(_, notice)| notice.clone())
        .collect();
    Ok(plan)
}
pub fn apply(store: &Store, plan: ImportPlan) -> Result<usize> {
    store.import_catalog(CatalogExport {
        schema_version: 1,
        hosts: plan.hosts,
    })
}
