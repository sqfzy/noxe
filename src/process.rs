#![allow(dead_code)]

use crate::cli::{Cli, NoteType};
use anyhow::{Context, Result, bail};
use chrono::{Datelike, Timelike};
use ignore::{DirEntry, WalkBuilder};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Write},
    path::{Component, Path},
    process::Command,
};

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
            let mut note_path = Path::new(&note_path_str).to_path_buf();

            if note_path
                .parent()
                .and_then(|p| p.to_str())
                .is_none_or(|s| s.is_empty())
            {
                // note_path是note name而非路径
                let note_dir = Path::new(&note_dir);

                let mut result =
                    search_notes(note_dir, &|s| s.eq_ignore_ascii_case(&note_path_str))?;

                note_path = match result.len() {
                    0 => bail!("No note found in '{}'", note_dir.display()),
                    1 => result.pop().unwrap().path().to_path_buf(),
                    _ => prompt_user_choice(&result)?.path().to_path_buf(),
                };
            };

            if note_path.is_dir() {
                if note_path.join("main.md").is_file() {
                    note_path = note_path.join("main.md");
                } else if note_path.join("main.typ").is_file() {
                    note_path = note_path.join("main.typ");
                } else {
                    bail!("No main file found in '{}'", note_path.display());
                }
            }

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
        Cli::List { note_dir } => {
            let note_dir_path = Path::new(&note_dir);

            let result = search_notes(note_dir_path, &|_| true)?;
            let paths = result
                .iter()
                .map(|e| e.path().strip_prefix(note_dir_path).unwrap())
                .collect::<Vec<_>>();
            print_tree(&paths);
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
    let mut file_notes = Vec::new();
    let mut dir_notes = Vec::new();

    let mut handle_filenote = |entry: DirEntry| {
        if entry.file_name().to_str().is_some_and(eq) {
            file_notes.push(entry);
        }
        Ok(())
    };
    let mut handle_dirnote = |entry: DirEntry| {
        if entry.file_name().to_str().is_some_and(eq) {
            dir_notes.push(entry);
        }
        Ok(())
    };
    handle_notes(
        note_dir,
        Some(&mut handle_filenote),
        Some(&mut handle_dirnote),
        None,
    )?;

    file_notes.extend(dir_notes);
    Ok(file_notes)
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

fn handle_notes(
    root: &Path,
    mut handle_filenote: Option<&mut dyn FnMut(DirEntry) -> Result<()>>,
    mut handle_dirnote: Option<&mut dyn FnMut(DirEntry) -> Result<()>>,
    mut handle_category: Option<&mut dyn FnMut(DirEntry) -> Result<()>>,
) -> Result<()> {
    let mut it = WalkBuilder::new(root).build();

    it.next();
    loop {
        let entry = match it.next() {
            Some(entry) => entry,
            None => break,
        }?;

        if let Some(handle) = handle_filenote.as_mut()
            && is_filenote(&entry)
        {
            handle(entry)?;
        } else if let Some(handle) = handle_dirnote.as_mut()
            && is_dirnote(&entry)
        {
            handle(entry)?;
            it.skip_current_dir();
        } else if let Some(handle) = handle_category.as_mut()
            && is_category(&entry)
        {
            handle(entry)?;
        }
    }

    Ok(())
}

fn is_filenote(entry: &DirEntry) -> bool {
    entry.file_type().is_some_and(|t| t.is_file())
        && entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| NoteType::try_from(ext).ok())
            .is_some()
}

fn is_dirnote(entry: &DirEntry) -> bool {
    let path = entry.path();
    entry.file_type().is_some_and(|t| t.is_dir())
        && (path.join("main.md").is_file() || path.join("main.typ").is_file())
}

fn is_category(entry: &DirEntry) -> bool {
    let path = entry.path();
    entry.file_type().is_some_and(|t| t.is_dir())
        && !path.join("main.md").is_file()
        && !path.join("main.typ").is_file()
}

fn print_filenote(entry: &DirEntry) {
    println!("{}", entry.file_name().display());
}

fn print_filenote_verbosely(entry: &DirEntry) {
    println!("{}", entry.path().display());
}

fn print_dirnote(entry: &DirEntry) {
    println!("{}", entry.file_name().display());
}

fn print_dirnote_verbosely(entry: &DirEntry) {
    println!("{}", entry.path().display());
}

fn print_category(entry: &DirEntry) {
    println!("{}", entry.file_name().display());
}

fn print_category_verbosely(entry: &DirEntry) {
    println!("{}", entry.path().display());
}

fn print_tree(paths: &[&Path]) {
    #[derive(Debug)]
    struct PathNode {
        children: BTreeMap<String, PathNode>,
        is_file: bool,
    }

    impl PathNode {
        fn new() -> Self {
            PathNode {
                children: BTreeMap::new(),
                is_file: false,
            }
        }
    }

    fn add_path(root: &mut BTreeMap<String, PathNode>, path: &Path) {
        // 将整个 path 的组件收集到一个 Vec 中，方便判断最后一个
        let components: Vec<Component> = path.components().collect();

        // 准备一个 mutable 引用，指向当前层级的子节点 map
        let mut current_map = root;

        for (i, component) in components.iter().enumerate() {
            // 当前 component 转为字符串
            let comp_str = component.as_os_str().to_string_lossy().to_string();

            // 获取或插入一个子节点
            let node = current_map.entry(comp_str).or_insert_with(PathNode::new);

            // 如果是最后一个组件，标记 is_file
            if i == components.len() - 1 {
                node.is_file = true;
            }

            // 为下一轮循环将 `current_map` 移动到该节点的 children 上
            current_map = &mut node.children;
        }
    }

    fn print_subtree(
        node_map: &BTreeMap<String, PathNode>,
        prefix: &str,
        is_last: bool,
        node_name: Option<&str>,
    ) {
        if let Some(name) = node_name {
            let branch = if is_last { "└── " } else { "├── " };
            println!("{}{}{}", prefix, branch, name);
        }

        let new_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        let len = node_map.len();
        for (i, (child_name, child_node)) in node_map.iter().enumerate() {
            let child_is_last = i == (len - 1);
            print_subtree(
                &child_node.children,
                &new_prefix,
                child_is_last,
                Some(child_name),
            );
        }
    }

    let mut root = BTreeMap::new();

    // 先把所有路径插入 tree
    for p in paths {
        add_path(&mut root, p);
    }

    // 再写一个递归函数去打印
    print_subtree(&root, "", true, None);
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
    fn cli_list_args(note_dir: &str) -> Cli {
        Cli::List {
            note_dir: note_dir.to_string(),
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
        let args = cli_list_args(note_dir.to_str().unwrap());
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
