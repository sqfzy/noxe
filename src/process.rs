use crate::cli::{Cli, NoteType};
use anyhow::{Context, Result, bail};
use chrono::{Datelike, Timelike};
use serde::Deserialize;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};
use walkdir::{DirEntry, WalkDir};

thread_local! {
    static TOP_DIRS_IN_NOTE_DIR: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn process_command(args: Cli) -> Result<()> {
    match args {
        Cli::New {
            note_path,
            note_author,
            note_keywords,
            mut note_type,
            mut single_file,
            note_template,
            note_with_metadata,
        } => {
            let note_path = Path::new(&note_path);

            // 如果note_path包含扩展名，则表明是单文件
            if let Some(ext) = note_path.extension().and_then(|ext| ext.to_str())
                && let Ok(t) = NoteType::try_from(ext)
            {
                note_type = t;
                single_file = true;
            }

            let note_name = note_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow::anyhow!("Failed to parse note name"))?;

            // Check if the note already exists
            if fs::metadata(note_path).is_ok() {
                bail!("Note '{}' already exists", note_path.display());
            }

            let main_path = if single_file {
                note_path.to_path_buf()
            } else {
                note_path.join(format!("main.{}", note_type))
            };

            let mut main_file_data = String::new();

            // Optionally add metadata
            if note_with_metadata {
                main_file_data.push_str(&metadata(
                    note_name,
                    note_author.as_ref(),
                    note_type,
                    &note_keywords,
                ));
            }

            let note_template = if let Some(path) = note_template {
                load_note_template(&path)?
            } else {
                Default::default()
            };

            // Create the note template
            if !single_file {
                create_note_template(note_path, &note_template)?;
            }

            // Add main file data
            if matches!(note_type, NoteType::Typ)
                && let Some(main_typ) = &note_template.main_typ
            {
                main_file_data.push_str(main_typ);
            } else if matches!(note_type, NoteType::Md)
                && let Some(main_md) = &note_template.main_md
            {
                main_file_data.push_str(main_md);
            }

            // Create the main file and write data
            fs::write(&main_path, main_file_data)
                .with_context(|| format!("Failed to create main file '{}'", main_path.display()))?;

            println!("Note '{}' created successfully!", note_path.display());
        }
        Cli::Preview {
            note_path: note_path_str,
            note_dir,
        } => {
            let note_path = Path::new(&note_path_str);

            let note_path = if note_path
                .parent()
                .and_then(|p| p.to_str())
                .is_some_and(|s| !s.is_empty())
            {
                if note_path.is_dir() {
                    if note_path.join("main.md").is_file() {
                        note_path.join("main.md")
                    } else if note_path.join("main.typ").is_file() {
                        note_path.join("main.typ")
                    } else {
                        bail!("No main file found in '{}'", note_path.display());
                    }
                } else {
                    note_path.to_path_buf()
                }
            } else {
                search_note(Path::new(&note_dir), &note_path_str)?
            };

            let note_type = if let Some(ext) = note_path.extension().and_then(|ext| ext.to_str())
                && let Ok(note_type) = NoteType::try_from(ext)
            {
                note_type
            } else {
                bail!("Failed to parse note type from '{}'", note_path.display());
            };

            preview_note(&note_path, note_type)?;
            print!("Previewing note '{}'", note_path.display());
        }
    }

    Ok(())
}

/* `New` command helper */

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PathContent {
    Directory(HashMap<String, PathContent>), // 子目录
    File(String),                            // 文件内容
}

#[derive(Debug, Deserialize)]
struct NoteTemplate {
    paths: HashMap<String, PathContent>, // 顶层路径
    #[serde(rename = "main.typ")]
    main_typ: Option<String>,
    #[serde(rename = "main.md")]
    main_md: Option<String>,
}

impl Default for NoteTemplate {
    fn default() -> Self {
        let mut paths = HashMap::new();

        paths.insert("images".to_string(), PathContent::Directory(HashMap::new()));
        paths.insert(
            "chapter".to_string(),
            PathContent::Directory(HashMap::new()),
        );
        paths.insert(
            "bibliography".to_string(),
            PathContent::Directory(HashMap::new()),
        );

        NoteTemplate {
            paths,
            main_typ: None,
            main_md: None,
        }
    }
}

fn create_note_template(note_path: &Path, template: &NoteTemplate) -> Result<()> {
    // 递归创建目录和文件
    fn create_paths(note_dir: &Path, content: &HashMap<String, PathContent>) -> Result<()> {
        for (name, path_content) in content {
            let current_path = note_dir.join(name);

            match path_content {
                PathContent::Directory(sub_content) => {
                    fs::create_dir_all(&current_path).with_context(|| {
                        format!("Failed to create directory '{}'", current_path.display())
                    })?;
                    create_paths(&current_path, sub_content)?;
                }
                PathContent::File(file_content) => {
                    if let Some(parent) = current_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("Failed to create parent directory '{}'", parent.display())
                        })?;
                    }
                    let mut file = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&current_path)
                        .with_context(|| {
                            format!("Failed to create file '{}'", current_path.display())
                        })?;
                    file.write_all(file_content.as_bytes()).with_context(|| {
                        format!("Failed to write to file '{}'", current_path.display())
                    })?;
                }
            }
        }
        Ok(())
    }

    for top_dir in template.paths.keys() {
        TOP_DIRS_IN_NOTE_DIR.with_borrow_mut(|top_dirs| {
            top_dirs.push(top_dir.clone());
        });
    }

    create_paths(note_path, &template.paths)?;

    Ok(())
}

fn load_note_template(file_path: &str) -> Result<NoteTemplate> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read template file '{}'", file_path))?;
    let template: NoteTemplate = serde_yml::from_str(&content)
        .with_context(|| format!("Failed to parse template file '{}'", file_path))?;
    Ok(template)
}

fn metadata(
    note_name: &str,
    note_author: Option<&String>,
    note_type: NoteType,
    keywords: &[String],
) -> String {
    let keywords = keywords.join(", ");
    let now = chrono::Local::now();

    match note_type {
        NoteType::Md => {
            let mut md_metadata = String::from("---\n");
            md_metadata.push_str(&format!("title: \"{}\"\n", note_name));
            if let Some(author) = note_author {
                md_metadata.push_str(&format!("author: \"{}\"\n", author));
            }
            if !keywords.is_empty() {
                md_metadata.push_str(&format!("keywords: [{}]\n", keywords));
            }
            md_metadata.push_str(&format!(
                "date: \"{}\"\n---\n\n",
                now.format("%Y-%m-%d %H:%M:%S")
            ));
            md_metadata
        }
        NoteType::Typ => {
            let mut typ_metadata = format!("#set document(title: \"{}\"", note_name);
            if let Some(author) = note_author {
                typ_metadata.push_str(&format!(", author: \"{}\"", author));
            }
            if !keywords.is_empty() {
                typ_metadata.push_str(&format!(", keywords: ({})", keywords));
            }
            typ_metadata.push_str(&format!(
                ", date: datetime(year: {}, month: {}, day: {}, hour: {}, minute: {}, second: {}))\n\n",
                now.year(),
                now.month(),
                now.day(),
                now.hour(),
                now.minute(),
                now.second()
            ));
            typ_metadata
        }
    }
}

/* `Preview` command helper */

fn search_note(note_dir: &Path, note_name: &str) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    TOP_DIRS_IN_NOTE_DIR.with_borrow(|top_dirs| {
        if !note_name.ends_with(".md") && !note_name.ends_with(".typ") {
            // 递归搜索note_dir中的以note_name为名的文件夹或文件
            for entry in WalkDir::new(note_dir)
                .min_depth(1)
                .into_iter()
                .filter_entry(|e| {
                    find_main_file_depth_zero(e).is_none()
                        && !top_dirs
                            .iter()
                            .any(|dir| e.file_name().to_str().is_some_and(|s| s == dir)) // 保留除top_dirs之外的文件夹
                })
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case(note_name))
                        || e.path()
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case(note_name))
                })
            {
                if entry.file_type().is_dir()
                    && let Some(main_path) = find_main_file_depth_one(&entry)
                {
                    candidates.push(main_path);
                } else {
                    candidates.push(entry.path().to_path_buf());
                }
            }
        } else {
            // 递归搜索note_dir中的以note_name为名的文件，跳过包含main.*文件的文件夹
            for entry in WalkDir::new(note_dir)
                .min_depth(1)
                .into_iter()
                .filter_entry(|e| {
                    find_main_file_depth_one(e).is_none()
                        && !top_dirs
                            .iter()
                            .any(|dir| e.file_name().to_str().is_some_and(|s| s == dir))
                })
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case(note_name))
                })
            {
                candidates.push(entry.path().to_path_buf());
            }
        }
    });

    let note_path = match candidates.len() {
        0 => bail!("No note found in '{}'", note_dir.display()),
        1 => candidates[0].clone(),
        _ => prompt_user_choice(&candidates)?,
    };

    Ok(note_path)
}

fn find_main_file_depth_one(dir: &DirEntry) -> Option<PathBuf> {
    let path = dir.path();
    let candidates = ["main.md", "main.typ"];

    candidates
        .iter()
        .map(|file_name| path.join(file_name))
        .find(|path| path.is_file())
}

fn find_main_file_depth_zero(dir: &DirEntry) -> Option<PathBuf> {
    let path = dir.path().parent()?;
    let candidates = ["main.md", "main.typ"];

    candidates
        .iter()
        .map(|file_name| path.join(file_name))
        .find(|path| path.is_file())
}

fn prompt_user_choice(candidates: &[PathBuf]) -> Result<PathBuf> {
    eprintln!("Multiple matches found:");
    for (i, candidate) in candidates.iter().enumerate() {
        eprintln!("{}. {}", i + 1, candidate.display());
    }
    eprint!("Enter the number of the note to preview (default is 1): ");
    io::stdout()
        .flush()
        .with_context(|| "Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .with_context(|| "Failed to read user input")?;
    let choice = input.trim().parse::<usize>().unwrap_or(1);

    if choice == 0 || choice > candidates.len() {
        bail!("Choice out of range");
    }

    Ok(candidates[choice - 1].clone())
}

fn preview_note(note_path: &Path, note_type: NoteType) -> Result<()> {
    match note_type {
        NoteType::Md => {
            println!(r#"Running "glow {}""#, note_path.display());
            Command::new("glow")
                .arg(note_path)
                .status()
                .with_context(|| {
                    format!(
                        "Error: Failed to preview markdown note '{}'",
                        note_path.display()
                    )
                })?;
        }
        NoteType::Typ => {
            let root = note_path.parent().unwrap();
            println!(r#"Running "tinymist preview {}""#, note_path.display());
            Command::new("tinymist")
                .arg("preview")
                .arg("--root")
                .arg(root)
                .arg(note_path)
                .status()
                .with_context(|| {
                    format!(
                        "Error: Failed to preview typora note '{}'",
                        note_path.display()
                    )
                })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, NoteType};
    use std::fs;
    use tempfile::tempdir;

    /// Helper to build Cli::New arguments quickly
    fn cli_new_args(note_path: &str, single_file: bool, note_type: NoteType) -> Cli {
        Cli::New {
            note_path: note_path.to_string(),
            note_author: Some("TestAuthor".to_string()),
            note_keywords: vec!["keyword1".to_string(), "keyword2".to_string()],
            note_type,
            single_file,
            note_template: None,
            note_with_metadata: true,
        }
    }

    /// Helper to build Cli::Preview arguments quickly
    fn cli_preview_args(note_path: &str, note_dir: &str) -> Cli {
        Cli::Preview {
            note_path: note_path.to_string(),
            note_dir: note_dir.to_string(),
        }
    }

    /// Test creating a single-file .md note
    #[test]
    fn test_new_single_file_md() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let note_file_path = temp_dir.path().join("MySingleMdNote.md");
        let note_file_str = note_file_path.to_str().unwrap();

        // Build arguments and run
        let args = cli_new_args(note_file_str, true, NoteType::Md);
        let result = process_command(args);

        // Verify success
        assert!(
            result.is_ok(),
            "Expected creation to succeed for single-file .md"
        );

        // Check the actual file
        assert!(note_file_path.exists(), "Expected .md file to be created");

        // Check metadata
        let content =
            fs::read_to_string(&note_file_path).expect("Failed to read the newly created .md file");
        assert!(
            content.contains("title: \"MySingleMdNote\""),
            "Should contain a 'title' metadata"
        );
        assert!(
            content.contains("author: \"TestAuthor\""),
            "Should contain 'author' metadata"
        );
        assert!(
            content.contains("keywords: [keyword1, keyword2]"),
            "Should contain 'keywords'"
        );
    }

    /// Test creating a multi-file .typ note with default template
    #[test]
    fn test_new_multi_file_typ() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let note_folder_path = temp_dir.path().join("MyMultiTypNote");
        let note_folder_str = note_folder_path.to_str().unwrap();

        // Build arguments and run
        let args = cli_new_args(note_folder_str, false, NoteType::Typ);
        let result = process_command(args);

        // Verify success
        assert!(
            result.is_ok(),
            "Expected creation to succeed for multi-file .typ"
        );

        // We expect a folder "MyMultiTypNote"
        assert!(
            note_folder_path.is_dir(),
            "Expected a directory to be created"
        );

        // Check for main.typ
        let main_typ_path = note_folder_path.join("main.typ");
        assert!(
            main_typ_path.exists(),
            "Expected main.typ file in the new folder"
        );

        // Check default subfolders
        let images_path = note_folder_path.join("images");
        let chapter_path = note_folder_path.join("chapter");
        let bibliography_path = note_folder_path.join("bibliography");
        assert!(images_path.is_dir(), "Expected an 'images' subdirectory");
        assert!(chapter_path.is_dir(), "Expected a 'chapter' subdirectory");
        assert!(
            bibliography_path.is_dir(),
            "Expected a 'bibliography' subdirectory"
        );
    }

    /// Test creating a note that already exists
    #[test]
    fn test_note_already_exists() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let note_file_path = temp_dir.path().join("DuplicateNote.md");
        let note_file_str = note_file_path.to_str().unwrap();

        // First creation should succeed
        {
            let args = cli_new_args(note_file_str, true, NoteType::Md);
            let result = process_command(args);
            assert!(result.is_ok(), "First creation should succeed");
        }

        // Second creation with the same path should fail
        {
            let args = cli_new_args(note_file_str, true, NoteType::Md);
            let result = process_command(args);
            assert!(
                result.is_err(),
                "Second creation with same path should fail"
            );
            let err_msg = format!("{}", result.unwrap_err());
            assert!(
                err_msg.contains("already exists"),
                "Error should mention 'already exists'"
            );
        }
    }

    /// Test previewing a note that doesn't exist
    #[test]
    fn test_preview_note_not_found() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let temp_dir_str = temp_dir.path().to_str().unwrap();

        // Attempt to preview a non-existent note
        let args = cli_preview_args("NonExistentNote", temp_dir_str);
        let result = process_command(args);

        assert!(
            result.is_err(),
            "Preview should fail for a non-existent note"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("No note found"),
            "Error should mention 'No note found'"
        );
    }

    /// Test previewing an existing .md note
    ///
    /// We create a single-file .md note first, then try to preview it.
    #[test]
    fn test_preview_existing_md() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let note_file_path = temp_dir.path().join("PreviewMe.md");
        let note_file_str = note_file_path.to_str().unwrap();
        let temp_dir_str = temp_dir.path().to_str().unwrap();

        // 1. Create a single-file .md note
        {
            let args = cli_new_args(note_file_str, true, NoteType::Md);
            let result = process_command(args);
            assert!(
                result.is_ok(),
                "Expected single-file .md creation to succeed"
            );
        }

        // 2. Attempt to preview that note
        {
            let args = cli_preview_args("PreviewMe.md", temp_dir_str);
            let result = process_command(args);

            // If `glow` is not installed in the test environment, the command might fail,
            // but logically, our code completes its logic up to the external call.
            // For the test environment, we typically just check that no internal error occurs.
            //
            // If you want to avoid actually calling external commands,
            // you could mock out `Command::new` for pure unit testing.
            assert!(
                result.is_ok(),
                "Preview of existing .md note should succeed"
            );
        }
    }
}
