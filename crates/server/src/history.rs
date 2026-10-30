//! Save versioning. `games/` is its own git repository (BoxInsertDesigner-games), and the server
//! keeps its history: every change is noted with the command behind it, and a game is committed
//! once its changes have settled for `QUIET`. Named versions are annotated tags
//! (`<slug>/v<n>-<name>`). A restore puts a game's files back as they were at a commit and
//! commits that, so history only ever grows. Commits are pushed to `origin` when there is one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::{Reason, now_ms};

/// How long a game's changes must settle before they're committed.
pub const QUIET: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Pending {
    first_ms: u64,
    last_ms: u64,
    lines: Vec<String>,
}

#[derive(Default)]
struct Status {
    last_commit_ms: BTreeMap<String, u64>,
    push_error: Option<String>,
    last_push_ms: Option<u64>,
}

pub struct History {
    dir: PathBuf,
    pending: Mutex<BTreeMap<String, Pending>>,
    status: Mutex<Status>,
    /// One git command at a time.
    git_lock: Mutex<()>,
}

fn err(e: impl std::fmt::Display) -> Reason {
    Reason::new("history", e.to_string())
}

impl History {
    pub fn new(dir: &Path) -> History {
        History { dir: dir.to_path_buf(), pending: Mutex::default(), status: Mutex::default(), git_lock: Mutex::default() }
    }

    /// Versioning is on when `games/` is a git repository.
    pub fn enabled(&self) -> bool {
        self.dir.join(".git").exists()
    }

    async fn git(&self, args: &[&str], stdin: Option<&str>) -> Result<String, Reason> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        if stdin.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }
        let mut child = cmd.spawn().map_err(err)?;
        if let Some(text) = stdin {
            use tokio::io::AsyncWriteExt;
            let mut si = child.stdin.take().unwrap();
            si.write_all(text.as_bytes()).await.map_err(err)?;
        }
        let out = child.wait_with_output().await.map_err(err)?;
        if !out.status.success() {
            return Err(err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Record a change to a game, for its next commit message.
    pub async fn note(&self, slug: &str, line: &str) {
        if !self.enabled() {
            return;
        }
        let now = now_ms();
        let mut p = self.pending.lock().await;
        let e = p.entry(slug.to_string()).or_insert_with(|| Pending { first_ms: now, ..Default::default() });
        e.last_ms = now;
        let line: String = line.lines().next().unwrap_or("").chars().take(200).collect();
        if e.lines.last() != Some(&line) {
            e.lines.push(line);
        }
    }

    /// Note games with changes nobody recorded (made while the server was down).
    pub async fn note_dirty(&self) {
        if !self.enabled() {
            return;
        }
        let Ok(out) = self.git(&["status", "--porcelain"], None).await else { return };
        let mut seen = std::collections::BTreeSet::new();
        for l in out.lines() {
            let path = l.get(3..).unwrap_or("").trim_matches('"');
            if let Some((slug, _)) = path.split_once('/') {
                if seen.insert(slug.to_string()) {
                    self.note(slug, "changes made while the server was stopped").await;
                }
            }
        }
    }

    /// Commit every game whose changes have settled. Returns the games committed.
    pub async fn flush_due(&self) -> Vec<String> {
        let due: Vec<String> = {
            let p = self.pending.lock().await;
            let now = now_ms();
            p.iter().filter(|(_, e)| now.saturating_sub(e.last_ms) >= QUIET.as_millis() as u64).map(|(k, _)| k.clone()).collect()
        };
        let mut done = vec![];
        for slug in due {
            match self.commit(&slug, None).await {
                Ok(Some(_)) => done.push(slug),
                Ok(None) => {}
                Err(e) => eprintln!("history {slug}: {}", e.message),
            }
        }
        if !done.is_empty() {
            self.push().await;
        }
        done
    }

    /// Commit a game's files now, with the noted changes (and `extra`) as the message.
    /// Returns the new commit, or None when nothing changed.
    pub async fn commit(&self, slug: &str, extra: Option<&str>) -> Result<Option<String>, Reason> {
        if !self.enabled() {
            return Ok(None);
        }
        let pending = self.pending.lock().await.remove(slug);
        let _g = self.git_lock.lock().await;
        let path = format!("{slug}/");
        self.git(&["add", "-A", "--", &path], None).await?;
        let staged = self.git(&["diff", "--cached", "--name-only", "--", &path], None).await?;
        if staged.trim().is_empty() {
            return Ok(None);
        }
        let mut lines: Vec<String> = pending.map(|p| p.lines).unwrap_or_default();
        if let Some(x) = extra {
            lines.insert(0, x.to_string());
        }
        if lines.is_empty() {
            lines.push("changes".into());
        }
        let subject = match lines.len() {
            1 => format!("{slug}: {}", lines[0]),
            n => format!("{slug}: {} (+{} more)", lines[0], n - 1),
        };
        let body = if lines.len() > 1 { lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n") } else { String::new() };
        let msg = format!("{}\n\n{body}\n", subject.chars().take(120).collect::<String>());
        self.git(&["commit", "-q", "-F", "-", "--", &path], Some(&msg)).await?;
        let sha = self.git(&["rev-parse", "HEAD"], None).await?.trim().to_string();
        self.status.lock().await.last_commit_ms.insert(slug.to_string(), now_ms());
        Ok(Some(sha))
    }

    /// Push to origin, if there is one; remember a failure for the UI.
    pub async fn push(&self) {
        let has_origin = self.git(&["remote"], None).await.map(|r| r.lines().any(|l| l == "origin")).unwrap_or(false);
        if !has_origin {
            return;
        }
        let r = {
            let _g = self.git_lock.lock().await;
            tokio::time::timeout(Duration::from_secs(60), self.git(&["push", "-q", "--follow-tags", "origin", "HEAD"], None)).await
        };
        let mut st = self.status.lock().await;
        st.last_push_ms = Some(now_ms());
        st.push_error = match r {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(e.message),
            Err(_) => Some("git push timed out".into()),
        };
        if let Some(e) = &st.push_error {
            eprintln!("history: push failed: {e}");
        }
    }

    /// Commits that touch a game, newest first, with their named versions.
    pub async fn log(&self, slug: &str, limit: usize) -> Result<Vec<Value>, Reason> {
        if !self.enabled() {
            return Ok(vec![]);
        }
        let _g = self.git_lock.lock().await;
        let n = format!("-n{limit}");
        let out = self.git(&["log", &n, "--format=%H%x1f%ct%x1f%s%x1f%b%x1e", "--", &format!("{slug}/")], None).await?;
        let tags = self.tags(slug).await?;
        Ok(out
            .split('\x1e')
            .filter_map(|rec| {
                let f: Vec<&str> = rec.trim_matches('\n').split('\x1f').collect();
                if f.len() < 4 {
                    return None;
                }
                let sha = f[0].trim();
                let subject = f[2].strip_prefix(&format!("{slug}: ")).unwrap_or(f[2]);
                let changes: Vec<&str> = f[3].lines().filter_map(|l| l.strip_prefix("- ")).collect();
                let versions: Vec<&Value> = tags.iter().filter(|t| t["commit"] == sha).collect();
                Some(json!({
                    "commit": sha, "short": &sha[..sha.len().min(8)], "time_ms": f[1].trim().parse::<u64>().unwrap_or(0) * 1000,
                    "subject": subject, "changes": changes, "versions": versions,
                }))
            })
            .collect())
    }

    async fn tags(&self, slug: &str) -> Result<Vec<Value>, Reason> {
        let out = self
            .git(&["tag", "-l", &format!("{slug}/*"), "--format=%(refname:short)%1f%(*objectname)%1f%(objectname)%1f%(contents)%1e"], None)
            .await?;
        Ok(out
            .split('\x1e')
            .filter_map(|rec| {
                let f: Vec<&str> = rec.trim_matches('\n').split('\x1f').collect();
                if f.len() < 4 || f[0].is_empty() {
                    return None;
                }
                let commit = if f[1].is_empty() { f[2] } else { f[1] };
                let mut text = f[3].trim().splitn(2, '\n');
                Some(json!({ "tag": f[0], "commit": commit, "name": text.next().unwrap_or(""), "note": text.next().unwrap_or("").trim() }))
            })
            .collect())
    }

    /// Commit what's pending and tag it as a named version.
    pub async fn save_version(&self, slug: &str, name: &str, note: Option<&str>) -> Result<Value, Reason> {
        if !self.enabled() {
            return Err(Reason::new("no_history", "games/ isn't a git repository, so versions can't be saved"));
        }
        self.commit(slug, Some(&format!("before saving version '{name}'"))).await?;
        let n = self.tags(slug).await?.len() + 1;
        let label: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let tag = format!("{slug}/v{n}{}", if label.is_empty() { String::new() } else { format!("-{}", &label[..label.len().min(40)]) });
        let msg = match note {
            Some(x) if !x.trim().is_empty() => format!("{name}\n\n{}", x.trim()),
            _ => name.to_string(),
        };
        {
            let _g = self.git_lock.lock().await;
            self.git(&["tag", "-a", &tag, "-F", "-"], Some(&msg)).await?;
        }
        self.push().await;
        let sha = self.git(&["rev-parse", "HEAD"], None).await?.trim().to_string();
        Ok(json!({ "tag": tag, "commit": sha, "name": name }))
    }

    /// Put a game's files back as they were at `rev`, keeping what's there now in history first.
    /// The caller holds the spec write lock, validates, and announces the change.
    pub async fn restore_files(&self, slug: &str, rev: &str) -> Result<String, Reason> {
        if !self.enabled() {
            return Err(Reason::new("no_history", "games/ isn't a git repository"));
        }
        if rev.starts_with('-') {
            return Err(Reason::new("bad_rev", "a commit or version tag"));
        }
        self.commit(slug, Some("before a restore")).await?;
        let _g = self.git_lock.lock().await;
        let path = format!("{slug}/");
        let spec_at = format!("{rev}:{slug}/spec.toml");
        self.git(&["cat-file", "-e", &spec_at], None)
            .await
            .map_err(|_| Reason::new("no_version", format!("'{rev}' has no {slug}/spec.toml")).subject(slug))?;
        let subject = self.git(&["log", "-1", "--format=%s", rev], None).await?.trim().to_string();
        let short = self.git(&["rev-parse", "--short", &format!("{rev}^{{commit}}")], None).await?.trim().to_string();
        // Files added since go; files from then come back.
        let then: std::collections::BTreeSet<String> =
            self.git(&["ls-tree", "-r", "--name-only", rev, "--", &path], None).await?.lines().map(String::from).collect();
        for f in self.git(&["ls-files", "--", &path], None).await?.lines() {
            if !then.contains(f) {
                self.git(&["rm", "-q", "--", f], None).await?;
            }
        }
        self.git(&["checkout", rev, "--", &path], None).await?;
        Ok(format!("restored to {short} ({})", subject.strip_prefix(&format!("{slug}: ")).unwrap_or(&subject)))
    }

    pub async fn status(&self, slug: &str) -> Value {
        let p = self.pending.lock().await;
        let st = self.status.lock().await;
        json!({
            "enabled": self.enabled(),
            "pending_since_ms": p.get(slug).map(|e| e.first_ms),
            "pending_changes": p.get(slug).map_or(0, |e| e.lines.len()),
            "last_commit_ms": st.last_commit_ms.get(slug),
            "push_error": st.push_error,
            "last_push_ms": st.last_push_ms,
            "quiet_s": QUIET.as_secs(),
        })
    }
}
