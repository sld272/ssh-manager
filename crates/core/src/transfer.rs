use crate::{process, ssh::OpenSsh, *};
use chrono::Utc;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    File,
    Contents,
}
// OpenSSH sftp already escapes glob characters inside quotes; adding another backslash turns [test] into a different filename.
fn sftp_quote(s: &str) -> String {
    let mut result = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}
fn local_path(path: &Path) -> Result<PathBuf> {
    let p = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    model::validate_absolute_path(&p.to_string_lossy())?;
    Ok(p)
}
fn upload_tree(source: &Path, target: &str, lines: &mut Vec<String>) -> Result<()> {
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new("invalid_path", "文件名需要使用 UTF-8 编码"))?;
        let path = entry.path();
        model::validate_absolute_path(&path.to_string_lossy())?;
        let remote = format!("{}/{}", target.trim_end_matches('/'), name);
        let metadata = entry.file_type()?;
        // A symlink such as dist/private -> ~/.ssh must not recursively upload files outside the selected directory.
        error::require(
            !metadata.is_symlink(),
            "symlink_not_supported",
            "目录包含符号链接，请先整理为普通文件和目录",
        )?;
        if metadata.is_dir() {
            lines.push(format!("-mkdir {}", sftp_quote(&remote)));
            upload_tree(&path, &remote, lines)?;
        } else {
            error::require(
                metadata.is_file(),
                "special_file_not_supported",
                "只能传输普通文件和目录",
            )?;
            lines.push(format!(
                "put {} {}",
                sftp_quote(&path.to_string_lossy()),
                sftp_quote(&remote)
            ));
        }
    }
    Ok(())
}
impl OpenSsh<'_> {
    /// File destinations are complete filenames; contents destinations are directories. Neither is inferred from existence.
    pub fn upload(
        &self,
        host: &str,
        connection: Option<&str>,
        source: &Path,
        destination: &str,
        mode: TransferMode,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        model::validate_absolute_path(destination)?;
        let source = local_path(source)?;
        let metadata = std::fs::symlink_metadata(&source)?;
        error::require(
            !metadata.file_type().is_symlink(),
            "symlink_not_supported",
            "请选择普通文件或目录",
        )?;
        if let TransferMode::File = mode {
            error::require(
                metadata.is_file() && !destination.ends_with('/'),
                "invalid_transfer_target",
                "文件模式需要普通源文件和完整目标文件名",
            )?;
            return self.upload_file(host, connection, &source, &metadata, destination, options);
        }
        error::require(
            metadata.is_dir(),
            "invalid_transfer_source",
            "内容模式需要源目录",
        )?;
        let mut lines = vec![format!("mkdir {}", sftp_quote(destination))];
        upload_tree(&source, destination, &mut lines)?;
        let target_type = format!(
            "test -d {} && test ! -L {}",
            ssh::shell_quote(destination),
            ssh::shell_quote(destination)
        );
        let preflight = self.exec(host, connection, &target_type, None, None, options)?;
        // Destination exists: merge entries into this directory instead of creating another basename level.
        if preflight.successful() {
            lines.remove(0);
        } else if preflight.status != Status::CommandFailed {
            return Ok(preflight);
        }
        self.transfer(
            host,
            connection,
            lines,
            TransferReceipt {
                source: source.to_string_lossy().into(),
                destination: destination.into(),
                direction: "upload".into(),
                mode: "contents".into(),
                bytes: None,
                sha256: None,
            },
            options,
        )
    }
    /// Streams the file over one connection into a hidden sibling of the target, which replaces the target only
    /// once its SHA-256 matches, with the old file's mode and owner. An interrupted upload leaves the target as it was.
    fn upload_file(
        &self,
        host: &str,
        connection: Option<&str>,
        source: &Path,
        metadata: &std::fs::Metadata,
        destination: &str,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        use sha2::{Digest, Sha256};
        use std::{
            io::{Seek, SeekFrom},
            os::unix::fs::PermissionsExt,
        };
        let mut file = std::fs::File::open(source)?;
        let mut hasher = Sha256::new();
        let bytes = std::io::copy(&mut file, &mut hasher)?;
        let sha256: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        file.seek(SeekFrom::Start(0))?;
        // Exit 3: the target is unusable, 4: the target was left untouched, 5: writing it in place failed partway.
        let script = format!(
            r#"dest={dest}
# After a dropped connection the messages below have nowhere to go; carry on so the temporary file is removed.
trap '' PIPE
if [ -d "$dest" ] || [ -L "$dest" ]; then echo "目标是目录或符号链接，请提供完整的文件名" >&2; exit 3; fi
dir=${{dest%/*}}
tmp="$dir/.${{dest##*/}}.ssh-manager-{token}"
verify() {{ [ "$(sha256sum "$1" | cut -d' ' -f1)" = {sha256} ] || {{ echo "上传后的内容与本机文件不一致" >&2; return 1; }}; }}
if ! (umask 077 && : > "$tmp") 2>/dev/null; then
  # A directory this account cannot add files to may still hold a file it can overwrite.
  [ -f "$dest" ] && [ -w "$dest" ] || {{ echo "没有在目标位置写入文件的权限" >&2; exit 4; }}
  cat > "$dest" && verify "$dest" || exit 5
  exit 0
fi
cat > "$tmp" && verify "$tmp" || {{ rm -f "$tmp"; exit 4; }}
if [ -e "$dest" ]; then
  chmod --reference="$dest" "$tmp" || {{ rm -f "$tmp"; exit 4; }}
  # A file owned by another account keeps its owner only when written in place.
  if [ "$(stat -c %u:%g "$dest")" != "$(stat -c %u:%g "$tmp")" ] && ! chown --reference="$dest" "$tmp" 2>/dev/null; then
    cat "$tmp" > "$dest" && verify "$dest"; code=$?
    rm -f "$tmp"
    [ $code -eq 0 ] || exit 5
    exit 0
  fi
else
  chmod "$(printf %o $((0{mode:o} & ~0$(umask))))" "$tmp" || {{ rm -f "$tmp"; exit 4; }}
fi
mv -f "$tmp" "$dest" || {{ rm -f "$tmp"; exit 4; }}
"#,
            dest = ssh::shell_quote(destination),
            token = Uuid::new_v4().simple(),
            mode = metadata.permissions().mode() & 0o777,
        );
        let receipt = TransferReceipt {
            source: source.to_string_lossy().into(),
            destination: destination.into(),
            direction: "upload".into(),
            mode: "file".into(),
            bytes: Some(bytes),
            sha256: Some(sha256),
        };
        self.exec_kind(
            host,
            connection,
            &script,
            None,
            Some(process::Input::File(file)),
            options,
            "upload",
            |r, _, _| {
                if r.status == Status::CommandFailed {
                    r.status = Status::TransferFailed;
                    r.next_step = Some(
                        if r.exit_code == Some(5) {
                            "目标文件可能只写入了一部分，请检查后再继续"
                        } else {
                            "目标文件没有改动"
                        }
                        .into(),
                    );
                }
                let mut receipt = receipt;
                if !r.successful() {
                    receipt.sha256 = None;
                }
                r.transfer = Some(receipt);
                Ok(())
            },
        )
    }
    pub fn download(
        &self,
        host: &str,
        connection: Option<&str>,
        source: &str,
        destination: &Path,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        model::validate_absolute_path(source)?;
        let destination = local_path(destination)?;
        // get remote /existing-directory would otherwise choose a filename implicitly.
        error::require(
            !destination.is_dir() && !destination.is_symlink(),
            "invalid_transfer_target",
            "下载需要完整的本地文件名，不能是目录或符号链接",
        )?;
        let probe = self.exec(
            host,
            connection,
            &format!(
                "test -f {} && test ! -L {}",
                ssh::shell_quote(source),
                ssh::shell_quote(source)
            ),
            None,
            None,
            options,
        )?;
        if !probe.successful() {
            return Ok(probe);
        }
        // Download into a sibling temporary file, so an interrupted transfer never replaces a complete local file.
        let parent = destination
            .parent()
            .ok_or_else(|| Error::new("invalid_path", "下载目录无效"))?;
        let staging = tempfile::NamedTempFile::new_in(parent)?;
        let lines = vec![format!(
            "get {} {}",
            sftp_quote(source),
            sftp_quote(&staging.path().to_string_lossy())
        )];
        let mut result = self.transfer(
            host,
            connection,
            lines,
            TransferReceipt {
                source: source.into(),
                destination: destination.to_string_lossy().into(),
                direction: "download".into(),
                mode: "file".into(),
                bytes: None,
                sha256: None,
            },
            options,
        )?;
        if result.successful()
            && let Err(e) = staging.persist(&destination)
        {
            result.status = Status::TransferFailed;
            result.next_step = Some(format!("已下载，但无法保存到目标位置：{}", e.error));
        }
        let h = self.store.host(host)?;
        self.store
            .record(&result, &h.name, &options.caller, options.reason.as_deref())?;
        Ok(result)
    }
    fn transfer(
        &self,
        host: &str,
        connection: Option<&str>,
        lines: Vec<String>,
        receipt: TransferReceipt,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        let h = self.store.host(host)?;
        let c = h.connection(connection)?;
        let started_at = Utc::now();
        let clock = Instant::now();
        let id = Uuid::new_v4();
        let session = self.session(&h, c)?;
        let prepared = clock.elapsed();
        let mut cmd = self.command("sftp", &session);
        // -b enables fail-fast batch commands; explicitly disable its implicit BatchMode to permit our askpass broker.
        cmd.args(["-oBatchMode=no", "-b", "-", "--", &session.alias]);
        self.store.begin(
            id,
            &h,
            c.id,
            &receipt.direction,
            started_at,
            &options.caller,
            options.reason.as_deref(),
        )?;
        let raw = process::run(
            cmd,
            Some(process::Input::Bytes(
                format!(
                    "{}\n",
                    lines
                        .iter()
                        .map(|line| if let Some(rest) = line.strip_prefix('-') {
                            format!("-@{rest}")
                        } else {
                            format!("@{line}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                )
                .into_bytes(),
            )),
            options.timeout,
            options.cancel.clone(),
            options.output_limit - options.output_limit / 2,
            options.output_limit / 2,
            None,
        )?;
        let code = raw.code;
        let interrupted = raw.termination.is_some();
        let mut result = self.result(
            &h,
            c,
            &receipt.direction,
            id,
            started_at,
            prepared,
            clock.elapsed(),
            raw,
            &session.broker,
            options.output_limit,
            None,
        );
        result.transfer = Some(receipt);
        if !interrupted && code == Some(0) {
            result.status = Status::Succeeded;
            result.phase = ExecutionPhase::Completed;
            result.exit_code = Some(0);
            result.next_step = None;
        } else if !interrupted
            && !matches!(
                result.status,
                Status::HostKeyChanged | Status::AuthenticationFailed | Status::NeedsInteraction
            )
        {
            result.status = Status::TransferFailed;
            result.phase = ExecutionPhase::Unknown;
            result.next_step = Some("传输未完成，目标可能包含部分内容，请检查后再继续".into());
        }
        self.store
            .record(&result, &h.name, &options.caller, options.reason.as_deref())?;
        Ok(result)
    }
}
