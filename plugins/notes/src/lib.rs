use async_trait::async_trait;
use discord_assist_plugin_api::{Plugin, PluginError};
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::prelude::Context;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct NotesPlugin {
    vault_path: PathBuf,
}

impl NotesPlugin {
    pub fn new(vault_path: &str) -> Self {
        Self {
            vault_path: PathBuf::from(vault_path),
        }
    }

    async fn handle_search(&self, query: &str) -> Result<String, PluginError> {
        let files = walk_md_files(&self.vault_path).await?;
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for path in &files {
            if results.len() >= 10 {
                break;
            }

            let rel = path.strip_prefix(&self.vault_path).unwrap_or(path);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            if stem.to_lowercase().contains(&query_lower) {
                results.push(format!("- **{}**", rel.display()));
                continue;
            }

            let meta = match tokio::fs::metadata(path).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.len() > 1_048_576 {
                continue;
            }

            let content = match tokio::fs::read_to_string(path).await {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(pos) = content.to_lowercase().find(&query_lower) {
                let start = content[..pos]
                    .char_indices()
                    .rev()
                    .nth(30)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let end = content[pos..]
                    .char_indices()
                    .nth(query.len() + 30)
                    .map(|(i, _)| pos + i)
                    .unwrap_or(content.len());
                let snippet = content[start..end].replace('\n', " ");
                results.push(format!(
                    "- **{}**: ...{}...",
                    rel.display(),
                    escape_discord(&snippet)
                ));
            }
        }

        if results.is_empty() {
            Ok(format!("No notes matching \"{}\".", escape_discord(query)))
        } else {
            Ok(format!(
                "**Search: {}** ({} results)\n{}",
                escape_discord(query),
                results.len(),
                results.join("\n")
            ))
        }
    }

    async fn handle_read(&self, name: &str) -> Result<String, PluginError> {
        let name_lower = name.to_lowercase();
        let files = walk_md_files(&self.vault_path).await?;

        let found = files.iter().find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase() == name_lower)
                .unwrap_or(false)
        });

        let path = match found {
            Some(p) => p,
            None => return Ok(format!("Note \"{}\" not found.", escape_discord(name))),
        };

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| PluginError::Other(format!("Failed to read note: {e}")))?;

        let rel = path.strip_prefix(&self.vault_path).unwrap_or(path);

        let truncated = if content.len() > 1900 {
            let end = content[..1900]
                .char_indices()
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}...\n*(truncated)*", &content[..end])
        } else {
            content
        };

        Ok(format!("**{}**\n{}", rel.display(), truncated))
    }

    async fn handle_recent(&self) -> Result<String, PluginError> {
        let files = walk_md_files(&self.vault_path).await?;

        let mut entries: Vec<(PathBuf, u64)> = Vec::new();
        for path in files {
            if let Ok(meta) = tokio::fs::metadata(&path).await {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                entries.push((path, mtime));
            }
        }

        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(10);

        if entries.is_empty() {
            return Ok("No notes found.".into());
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut msg = String::from("**Recent Notes**\n");
        for (path, mtime) in &entries {
            let rel = path.strip_prefix(&self.vault_path).unwrap_or(path);
            msg.push_str(&format!(
                "- {} ({})\n",
                rel.display(),
                format_relative_time(now, *mtime)
            ));
        }
        Ok(msg)
    }

    async fn handle_quick(
        &self,
        title: &str,
        content: &str,
        folder: Option<&str>,
    ) -> Result<String, PluginError> {
        let sanitized = sanitize_title(title);
        if sanitized.is_empty() {
            return Ok("Invalid title.".into());
        }

        let folder_name = folder.unwrap_or("Discord");
        if !validate_folder(folder_name) {
            return Ok("Invalid folder path.".into());
        }

        let canonical_vault = tokio::fs::canonicalize(&self.vault_path)
            .await
            .map_err(|e| PluginError::Other(format!("Vault path error: {e}")))?;

        let dir = canonical_vault.join(folder_name);

        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::Other(format!("Failed to create folder: {e}")))?;

        let canonical_dir = tokio::fs::canonicalize(&dir)
            .await
            .map_err(|e| PluginError::Other(format!("Path error: {e}")))?;

        if !canonical_dir.starts_with(&canonical_vault) {
            return Ok("Invalid folder path.".into());
        }

        let filename = format!("{}.md", sanitized);
        let file_path = canonical_dir.join(&filename);

        if tokio::fs::metadata(&file_path).await.is_ok() {
            return Ok(format!("Note \"{}\" already exists.", filename));
        }

        let date = today_iso();
        let body = format!("---\ncreated: {date}\n---\n\n{content}\n");

        tokio::fs::write(&file_path, &body)
            .await
            .map_err(|e| PluginError::Other(format!("Failed to write note: {e}")))?;

        let rel = file_path
            .strip_prefix(&canonical_vault)
            .unwrap_or(&file_path);
        Ok(format!("Created **{}**", rel.display()))
    }

    async fn handle_list(&self, folder: Option<&str>) -> Result<String, PluginError> {
        if let Some(f) = folder
            && !validate_folder(f)
        {
            return Ok("Invalid folder path.".into());
        }

        let dir = match folder {
            Some(f) => self.vault_path.join(f),
            None => self.vault_path.clone(),
        };

        let canonical_vault = tokio::fs::canonicalize(&self.vault_path)
            .await
            .map_err(|e| PluginError::Other(format!("Vault path error: {e}")))?;

        let canonical_dir = match tokio::fs::canonicalize(&dir).await {
            Ok(p) => p,
            Err(_) => return Ok("Folder not found.".into()),
        };

        if !canonical_dir.starts_with(&canonical_vault) {
            return Ok("Invalid folder path.".into());
        }

        let mut entries = tokio::fs::read_dir(&canonical_dir)
            .await
            .map_err(|_| PluginError::Other("Cannot read folder.".into()))?;

        let mut files = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md")
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                files.push(name.to_string());
            }
        }

        files.sort();
        let total = files.len();
        files.truncate(25);

        if files.is_empty() {
            return Ok("No notes found.".into());
        }

        let folder_display = folder.unwrap_or("vault root");
        let mut msg = format!("**Notes in {}** ({})\n", folder_display, total);
        for f in &files {
            msg.push_str(&format!("- {f}\n"));
        }
        if total > 25 {
            msg.push_str(&format!("*...and {} more*\n", total - 25));
        }
        Ok(msg)
    }
}

#[async_trait]
impl Plugin for NotesPlugin {
    fn name(&self) -> &str {
        "notes"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("notes")
            .description("Obsidian vault notes management")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "search",
                    "Search notes by filename or content",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "Search query",
                    )
                    .required(true),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "read",
                    "Display a note's content",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "name",
                        "Note name (without .md extension)",
                    )
                    .required(true),
                ),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "recent",
                "List 10 most recently modified notes",
            ))
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "quick",
                    "Create a quick note",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "title",
                        "Note title",
                    )
                    .required(true),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "content",
                        "Note content",
                    )
                    .required(true),
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "folder",
                    "Folder path (default: Discord)",
                )),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "list",
                    "List notes in a folder",
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "folder",
                    "Folder path (default: vault root)",
                )),
            )]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "notes" {
            return Ok(false);
        }

        // DM-only: reject if used in a server
        if command.guild_id.is_some() {
            let data = CreateInteractionResponseMessage::new()
                .content("Notes commands are only available in DMs.")
                .ephemeral(true);
            let builder = CreateInteractionResponse::Message(data);
            command
                .create_response(&ctx.http, builder)
                .await
                .map_err(PluginError::DiscordError)?;
            return Ok(true);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match subopt.name {
            "search" => {
                let query = extract_string_option(&subopt.value, "query")
                    .ok_or_else(|| PluginError::Other("Missing query".into()))?;
                self.handle_search(query).await?
            }
            "read" => {
                let name = extract_string_option(&subopt.value, "name")
                    .ok_or_else(|| PluginError::Other("Missing name".into()))?;
                self.handle_read(name).await?
            }
            "recent" => self.handle_recent().await?,
            "quick" => {
                let title = extract_string_option(&subopt.value, "title")
                    .ok_or_else(|| PluginError::Other("Missing title".into()))?;
                let content = extract_string_option(&subopt.value, "content")
                    .ok_or_else(|| PluginError::Other("Missing content".into()))?;
                let folder = extract_string_option(&subopt.value, "folder");
                self.handle_quick(title, content, folder).await?
            }
            "list" => {
                let folder = extract_string_option(&subopt.value, "folder");
                self.handle_list(folder).await?
            }
            _ => return Ok(false),
        };

        let data = CreateInteractionResponseMessage::new().content(content);
        let builder = CreateInteractionResponse::Message(data);
        command
            .create_response(&ctx.http, builder)
            .await
            .map_err(PluginError::DiscordError)?;
        Ok(true)
    }
}

fn extract_string_option<'a>(value: &'a ResolvedValue<'a>, name: &str) -> Option<&'a str> {
    if let ResolvedValue::SubCommand(opts) = value {
        opts.iter()
            .find(|o| o.name == name)
            .and_then(|o| match &o.value {
                ResolvedValue::String(s) => Some(*s),
                _ => None,
            })
    } else {
        None
    }
}

async fn walk_md_files(dir: &Path) -> Result<Vec<PathBuf>, PluginError> {
    let canonical_root = tokio::fs::canonicalize(dir)
        .await
        .map_err(|e| PluginError::Other(format!("Cannot resolve vault path: {e}")))?;
    let mut files = Vec::new();
    let mut stack = vec![canonical_root.clone()];

    while let Some(current) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&current).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.starts_with('.') {
                continue;
            }

            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            let path = entry.path();
            if file_type.is_dir() {
                if let Ok(canonical) = tokio::fs::canonicalize(&path).await
                    && canonical.starts_with(&canonical_root)
                {
                    stack.push(canonical);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("md")
                && let Ok(canonical) = tokio::fs::canonicalize(&path).await
                && canonical.starts_with(&canonical_root)
            {
                files.push(canonical);
            }
        }
    }

    Ok(files)
}

fn escape_discord(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('~', "\\~")
        .replace('|', "\\|")
        .replace('@', "\\@")
        .replace('<', "\\<")
}

fn validate_folder(name: &str) -> bool {
    !name.contains("..")
        && !name.starts_with('/')
        && !name.starts_with('\\')
        && !name.contains('\0')
}

fn sanitize_title(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '-' })
        .collect();

    let s = s.split_whitespace().collect::<Vec<_>>().join("-");

    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    result.trim_matches('-').to_string()
}

fn format_relative_time(now: u64, timestamp: u64) -> String {
    if timestamp == 0 || timestamp > now {
        return "just now".into();
    }
    let diff = now - timestamp;
    if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn today_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days = secs / 86400;
    let mut remaining = days;
    let mut year = 1970u32;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let leap = is_leap_year(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 12u32;
    for (i, &d) in month_days.iter().enumerate() {
        if remaining < d {
            month = i as u32 + 1;
            break;
        }
        remaining -= d;
    }

    let day = remaining + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_title_basic() {
        assert_eq!(sanitize_title("My Note Title"), "my-note-title");
    }

    #[test]
    fn test_sanitize_title_special_chars() {
        assert_eq!(sanitize_title("Hello! World? #2024"), "hello-world-2024");
    }

    #[test]
    fn test_sanitize_title_extra_spaces() {
        assert_eq!(sanitize_title("  lots   of   spaces  "), "lots-of-spaces");
    }

    #[test]
    fn test_sanitize_title_empty() {
        assert_eq!(sanitize_title("!!!"), "");
    }

    #[test]
    fn test_format_relative_time_minutes() {
        assert_eq!(format_relative_time(1000, 400), "10m ago");
    }

    #[test]
    fn test_format_relative_time_hours() {
        assert_eq!(format_relative_time(10000, 3000), "1h ago");
    }

    #[test]
    fn test_format_relative_time_days() {
        assert_eq!(format_relative_time(200000, 10000), "2d ago");
    }

    #[test]
    fn test_format_relative_time_future() {
        assert_eq!(format_relative_time(100, 200), "just now");
    }

    #[test]
    fn test_format_relative_time_zero() {
        assert_eq!(format_relative_time(100, 0), "just now");
    }

    #[test]
    fn test_today_iso_format() {
        let result = today_iso();
        assert_eq!(result.len(), 10);
        assert_eq!(&result[4..5], "-");
        assert_eq!(&result[7..8], "-");
        let year: u32 = result[..4].parse().unwrap();
        assert!(year >= 2024);
        let month: u32 = result[5..7].parse().unwrap();
        assert!((1..=12).contains(&month));
        let day: u32 = result[8..10].parse().unwrap();
        assert!((1..=31).contains(&day));
    }

    #[test]
    fn test_validate_folder() {
        assert!(validate_folder("Discord"));
        assert!(validate_folder("foo/bar"));
        assert!(!validate_folder("../evil"));
        assert!(!validate_folder("/absolute"));
        assert!(!validate_folder("foo/../bar"));
        assert!(!validate_folder("foo/.."));
        assert!(!validate_folder(".."));
    }

    #[test]
    fn test_sanitize_title_path_traversal() {
        assert_eq!(sanitize_title("../../etc/passwd"), "etc-passwd");
    }

    #[test]
    fn test_escape_discord() {
        assert_eq!(escape_discord("**bold**"), "\\*\\*bold\\*\\*");
        assert_eq!(escape_discord("@everyone"), "\\@everyone");
        assert_eq!(escape_discord("<@123>"), "\\<\\@123>");
        assert_eq!(escape_discord("normal text"), "normal text");
    }
}
