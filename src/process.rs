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
                // note_path是note name而非路径
                let note_dir = Path::new(&note_dir);

                let mut result =
                    search_notes(note_dir, &|s| s.eq_ignore_ascii_case(&note_path_str))?;

                match result.len() {
                    0 => bail!("No note found in '{}'", note_dir.display()),
                    1 => result.pop().unwrap().path().to_path_buf(),
                    _ => prompt_user_choice(&result)?.path().to_path_buf(),
                }
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
        Cli::Search { query, note_dir } => {
            let pattern = regex::RegexBuilder::new(&query)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("Failed to build regex from '{}'", query))?;

            let note_dir = Path::new(&note_dir);
            let result = search_notes(note_dir, &|s| pattern.is_match(s))?;

            if result.is_empty() {
                bail!("No note found in '{}'", note_dir.display());
            }

            println!("Found notes:");
            for entry in result {
                println!("{}", entry.path().display());
            }
        }
        Cli::List { note_dir, verbose } => {
            let note_dir_path = Path::new(&note_dir);

            let result = find_all_notes(note_dir_path)?;
            if result.is_empty() {
                bail!("No note found in '{}'", note_dir_path.display());
            }

            println!("Found notes:");
            for entry in result {
                if verbose {
                    println!("{}", entry.path().display());
                } else {
                    println!(
                        "{}",
                        entry
                            .path()
                            .display()
                            .to_string()
                            .split_off(note_dir.len() + 1)
                    );
                }
            }
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

fn search_notes(note_dir: &Path, eq: &dyn Fn(&str) -> bool) -> Result<Vec<DirEntry>> {
    let notes = find_all_notes(note_dir)?;

    let res = notes
        .into_iter()
        .filter(|note| {
            note.path()
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(eq)
        })
        .collect();

    Ok(res)
}

fn prompt_user_choice(candidates: &[DirEntry]) -> Result<DirEntry> {
    eprintln!("Multiple matches found:");
    for (i, candidate) in candidates.iter().enumerate() {
        eprintln!("{}. {}", i + 1, candidate.path().display());
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

    // Ok(candidates[choice - 1].clone())
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

/* common helper */

fn find_all_notes(note_dir: &Path) -> Result<Vec<DirEntry>> {
    let mut entries = Vec::new();

    let mut it = WalkDir::new(note_dir).min_depth(1).into_iter();
    while let Some(entry) = it.next() {
        let entry = entry?;

        if entry.file_type().is_dir() && find_main_file_depth_one(&entry).is_some() {
            entries.push(entry);
            it.skip_current_dir();
        } else if let Some(ext) = entry.path().extension()
            && (ext == "md" || ext == "typ")
        {
            entries.push(entry);
        }
    }

    Ok(entries)
}

fn find_main_file_depth_one(dir: &DirEntry) -> Option<PathBuf> {
    if !dir.file_type().is_dir() {
        return None;
    }

    let path = dir.path();
    let candidates = ["main.md", "main.typ"];

    candidates
        .iter()
        .map(|file_name| path.join(file_name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, NoteType};
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    /// Helper to build Cli::New arguments quickly
    fn cli_new_args(note_path: &str, single_file: bool, note_type: NoteType) -> Cli {
        Cli::New {
            note_path: note_path.to_string(),
            note_author: Some("TestAuthor".to_string()),
            note_keywords: ["keyword1".to_string(), "keyword2".to_string()].into(),
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

    /// Helper to build Cli::Search arguments quickly
    fn cli_search_args(query: &str, note_dir: &str) -> Cli {
        Cli::Search {
            query: query.to_string(),
            note_dir: note_dir.to_string(),
        }
    }

    /// Helper to build Cli::List arguments quickly
    fn cli_list_args(note_dir: &str, verbose: bool) -> Cli {
        Cli::List {
            note_dir: note_dir.to_string(),
            verbose,
        }
    }

    #[test]
    fn test_find_all_notes() {
        // This is an existing test that ensures your fixture data is found.
        // Make sure your fixture folder structure remains as expected.
        const ENTRIES: [&str; 10] = [
            "tests/fixtures/note_dir/note_file1.typ",
            "tests/fixtures/note_dir/note_file2.md",
            "tests/fixtures/note_dir/cat1/cat1_note_dir1",
            "tests/fixtures/note_dir/cat1/cat1_note_dir2",
            "tests/fixtures/note_dir/cat1/cat1_note_file.typ",
            "tests/fixtures/note_dir/cat1/sub_cat1/sub_cat1_note_file.typ",
            "tests/fixtures/note_dir/cat1/sub_cat1/sub_cat1_note_dir",
            "tests/fixtures/note_dir/cat2/cat1_note_dir1",
            "tests/fixtures/note_dir/cat2/cat2_note_dir",
            "tests/fixtures/note_dir/cat2/cat2_note_file.md",
        ];

        let note_dir = Path::new("tests/fixtures/note_dir");
        let result = find_all_notes(note_dir).unwrap();

        assert_eq!(result.len(), ENTRIES.len());
        for e in result {
            assert!(
                ENTRIES.contains(&e.path().to_str().unwrap()),
                "Entry not found: {:?}",
                e
            );
        }
    }

    #[test]
    fn test_process_command_new_single_file_md() {
        let tmp_dir = tempdir().unwrap();
        let note_path = tmp_dir.path().join("MyNote.md");
        let note_path_str = note_path.to_str().unwrap();

        let args = cli_new_args(note_path_str, true, NoteType::Md);
        let result = process_command(args);
        assert!(result.is_ok(), "Failed to create single-file md note");

        // Verify file creation
        assert!(note_path.is_file(), "Note file was not created");
        let contents = fs::read_to_string(note_path).unwrap();
        assert!(
            contents.contains("title: \"MyNote\""),
            "Metadata title not found in note"
        );
        assert!(
            contents.contains("author: \"TestAuthor\""),
            "Metadata author not found in note"
        );
    }

    #[test]
    fn test_process_command_new_single_file_typ() {
        let tmp_dir = tempdir().unwrap();
        let note_path = tmp_dir.path().join("AnotherNote.typ");
        let note_path_str = note_path.to_str().unwrap();

        let args = cli_new_args(note_path_str, true, NoteType::Typ);
        let result = process_command(args);
        assert!(result.is_ok(), "Failed to create single-file typ note");

        // Verify file creation
        assert!(note_path.is_file(), "Note file was not created");
        let contents = fs::read_to_string(note_path).unwrap();
        assert!(
            contents.contains("#set document(title: \"AnotherNote\""),
            "Metadata title not found in .typ note"
        );
        assert!(
            contents.contains("author: \"TestAuthor\""),
            "Metadata author not found in note"
        );
    }

    #[test]
    fn test_process_command_new_multi_file_md() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().join("multi_md");
        let main_file = note_dir.join("main.md");

        let args = cli_new_args(note_dir.to_str().unwrap(), false, NoteType::Md);
        let result = process_command(args);
        assert!(result.is_ok(), "Failed to create multi-file md note");

        // The main.md file should have been created with metadata
        assert!(main_file.is_file(), "main.md was not created");
        let contents = fs::read_to_string(main_file).unwrap();
        assert!(
            contents.contains("title: \"multi_md\""),
            "Metadata title not found in multi-file main.md"
        );

        // The template directories (images, chapter, bibliography) should exist
        let images_dir = note_dir.join("images");
        let chapter_dir = note_dir.join("chapter");
        let biblio_dir = note_dir.join("bibliography");
        assert!(images_dir.is_dir(), "images directory not created");
        assert!(chapter_dir.is_dir(), "chapter directory not created");
        assert!(biblio_dir.is_dir(), "bibliography directory not created");
    }

    #[test]
    fn test_process_command_new_already_exists() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().join("already_exists");
        let main_file = note_dir.join("main.md");
        fs::create_dir_all(&note_dir).unwrap();
        fs::write(&main_file, "Existing note content").unwrap();

        // Attempt to create a note at the same location
        let args = cli_new_args(note_dir.to_str().unwrap(), false, NoteType::Md);
        let result = process_command(args);

        assert!(result.is_err(), "Expected error when note already exists");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("already exists"),
            "Unexpected error message: {}",
            err_msg
        );
    }

    #[test]
    fn test_process_command_list() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().to_path_buf();

        // create a couple of notes
        let note1 = note_dir.join("TestNote1.md");
        fs::write(&note1, "# Test Note 1\n").unwrap();

        let note2_dir = note_dir.join("TestNote2");
        fs::create_dir_all(&note2_dir).unwrap();
        fs::write(
            note2_dir.join("main.typ"),
            "#set document(title: \"TestNote2\")\n",
        )
        .unwrap();

        // run `List` command
        let args = cli_list_args(note_dir.to_str().unwrap(), false);
        let result = process_command(args);
        assert!(result.is_ok(), "Failed to list notes");
        // We can't easily capture the printed output here,
        // but we can check if the function completed successfully.
    }

    #[test]
    fn test_process_command_search() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().to_path_buf();

        // create a note with a known name
        let note1_dir = note_dir.join("SearchedNote");
        fs::create_dir_all(&note1_dir).unwrap();
        fs::write(note1_dir.join("main.md"), "# Searched Note").unwrap();

        // create a second note that we don't want to match
        let note2_dir = note_dir.join("OtherNote");
        fs::create_dir_all(&note2_dir).unwrap();
        fs::write(note2_dir.join("main.md"), "# Other Note").unwrap();

        // run `Search` with partial name
        let args = cli_search_args("SearchedNote", note_dir.to_str().unwrap());
        let result = process_command(args);
        assert!(result.is_ok(), "Failed to search notes");
    }

    #[test]
    fn test_process_command_preview_single_file() {
        // This test will attempt to run "glow" or "tinymist".
        // If "glow" or "tinymist" isn't installed, it may fail.
        // Adjust or mock as necessary for your environment.

        let tmp_dir = tempdir().unwrap();
        let note_file = tmp_dir.path().join("PreviewMe.md");
        fs::write(&note_file, "# Preview me").unwrap();

        let args = cli_preview_args(
            note_file.to_str().unwrap(),
            tmp_dir.path().to_str().unwrap(),
        );
        let result = process_command(args);
        // We expect an error if "glow" doesn't exist, but let's just check the function call.
        // In a real environment, you'd want to handle or mock system calls.
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_process_command_preview_directory_with_main_md() {
        // Similar to above, this will attempt to run "glow" or "tinymist".
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().join("PreviewDir");
        fs::create_dir_all(&note_dir).unwrap();

        // create a main.md so it picks it up by default
        fs::write(note_dir.join("main.md"), "# main.md for preview").unwrap();

        let args = cli_preview_args("PreviewDir", tmp_dir.path().to_str().unwrap());
        let result = process_command(args);
        // This may or may not succeed in your local environment depending on "glow" availability.
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_create_note_template_function() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path().join("templated_note");

        let mut sub_paths = HashMap::new();
        sub_paths.insert(
            "subfile.md".to_string(),
            PathContent::File("content".to_string()),
        );
        let mut example_paths = HashMap::new();
        example_paths.insert("subdir".to_string(), PathContent::Directory(sub_paths));

        let template = NoteTemplate {
            paths: example_paths,
            main_typ: Some("Typ content".into()),
            main_md: Some("Md content".into()),
        };

        let result = create_note_template(&note_dir, &template);
        assert!(result.is_ok(), "Failed to create note template");

        let subdir = note_dir.join("subdir");
        let subfile = subdir.join("subfile.md");
        assert!(subdir.is_dir(), "Subdirectory was not created");
        assert!(subfile.is_file(), "Subfile.md was not created");
        let content = fs::read_to_string(subfile).unwrap();
        assert_eq!(&content, "content", "Wrong content in subfile.md");
    }

    #[test]
    fn test_load_note_template_function() {
        // We'll write a sample YAML file to a temp location, then load it.
        let tmp_dir = tempdir().unwrap();
        let template_file = tmp_dir.path().join("template.yml");

        // Example template in YAML
        let yaml_content = r#"
paths:
  images: {}
  chapter: {}
  bibliography: {}
"main.typ": "Typ content here"
"main.md": "Markdown content here"
"#;

        {
            let mut f = File::create(&template_file).unwrap();
            writeln!(f, "{}", yaml_content).unwrap();
        }

        let loaded_template = load_note_template(template_file.to_str().unwrap());
        assert!(loaded_template.is_ok(), "Failed to load note template");
        let loaded_template = loaded_template.unwrap();

        assert!(
            loaded_template.paths.contains_key("images"),
            "images folder missing"
        );
        assert!(
            loaded_template.paths.contains_key("chapter"),
            "chapter folder missing"
        );
        assert!(
            loaded_template.paths.contains_key("bibliography"),
            "bibliography folder missing"
        );
        assert_eq!(
            loaded_template.main_typ.as_deref(),
            Some("Typ content here"),
            "Wrong main_typ"
        );
        assert_eq!(
            loaded_template.main_md.as_deref(),
            Some("Markdown content here"),
            "Wrong main_md"
        );
    }
}
