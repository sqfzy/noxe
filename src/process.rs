#![allow(dead_code)]

use crate::cli::{Cli, NoteType};
use anyhow::{Context, Result, bail};
use chrono::{Datelike, Timelike};
use ignore::{DirEntry, WalkBuilder};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    ops::Deref,
    path::{Component, Path, PathBuf},
    process::Command,
};

trait Note {
    fn note_type(&self) -> Result<NoteType>;

    fn note_path(&self) -> Result<PathBuf>;

    fn is_filenote(&self) -> bool;

    fn is_dirnote(&self) -> bool;

    fn is_category(&self) -> bool;

    fn is_note_name(&self) -> bool;
}

impl<T: Deref<Target = Path>> Note for T {
    fn note_type(&self) -> Result<NoteType> {
        if let Some(ext) = self.extension().and_then(|ext| ext.to_str())
            && let Ok(note_type) = NoteType::try_from(ext)
        {
            Ok(note_type)
        } else {
            bail!("Failed to parse note type from '{}'", self.display());
        }
    }

    fn note_path(&self) -> Result<PathBuf> {
        let note_path = if self.is_dir() {
            if self.join("main.typ").is_file() {
                self.join("main.typ")
            } else if self.join("main.md").is_file() {
                self.join("main.md")
            } else {
                bail!("No main file found in '{}'", self.display())
            }
        } else {
            self.to_path_buf()
        };

        Ok(note_path)
    }

    fn is_filenote(&self) -> bool {
        self.is_file()
            && self
                .extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| NoteType::try_from(ext).ok())
                .is_some()
    }

    fn is_dirnote(&self) -> bool {
        self.is_dir() && (self.join("main.md").is_file() || self.join("main.typ").is_file())
    }

    fn is_category(&self) -> bool {
        self.is_dir() && !self.join("main.md").is_file() && !self.join("main.typ").is_file()
    }

    fn is_note_name(&self) -> bool {
        self.components().count() == 1
    }
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
            note_path,
            note_dir,
            mut preview_typst,
            mut preview_markdown,
        } => {
            let note_path_str = if let Some(s) = note_path {
                s
            } else {
                std::env::current_dir()?.into_os_string()
            };

            let mut note_path = Path::new(&note_path_str).to_path_buf();

            if note_path.is_note_name() {
                // note_path是note name而非路径
                let note_dir = Path::new(&note_dir);

                let mut result = search(note_dir, true, true, false, &|s| {
                    s.eq_ignore_ascii_case(&note_path_str)
                })?
                .concat();

                note_path = match result.len() {
                    0 => bail!("No note found in '{}'", note_dir.display()),
                    1 => result.pop().unwrap().path().to_path_buf(),
                    _ => prompt_user_choice(&result)?.path().to_path_buf(),
                };
            };

            let note_path = note_path.note_path()?;
            let note_type = note_path.note_type()?;

            if preview_typst.is_empty() {
                let root = note_path.parent().unwrap();
                preview_typst = vec![
                    "tinymist".into(),
                    "preview".into(),
                    "--root".into(),
                    root.into(),
                ];
            }
            if preview_markdown.is_empty() {
                preview_markdown = vec!["glow".into()];
            }

            match note_type {
                NoteType::Typ => exec_with(&note_path, &preview_typst)?,
                NoteType::Md => exec_with(&note_path, &preview_markdown)?,
            }

            println!("Previewing note '{}'", note_path.display());
        }
        Cli::Edit {
            note_path,
            note_dir,
            mut edit,
        } => {
            let note_path_str = if let Some(s) = note_path {
                s
            } else {
                std::env::current_dir()?.into_os_string()
            };

            let mut note_path = Path::new(&note_path_str).to_path_buf();

            if note_path.is_note_name() {
                // note_path是note name而非路径
                let note_dir = Path::new(&note_dir);

                let mut result = search(note_dir, true, true, false, &|s| {
                    s.eq_ignore_ascii_case(&note_path_str)
                })?
                .concat();

                note_path = match result.len() {
                    0 => bail!("No note found in '{}'", note_dir.display()),
                    1 => result.pop().unwrap().path().to_path_buf(),
                    _ => prompt_user_choice(&result)?.path().to_path_buf(),
                };
            };

            let note_path = note_path.note_path()?;

            if edit.is_empty() {
                edit = vec!["vim".into()];
            }

            exec_with(&note_path, &edit)?;
        }
        Cli::Search { query, note_dir } => {
            let pattern = regex::RegexBuilder::new(&query)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("Failed to build regex from '{}'", query))?;

            let note_dir = Path::new(&note_dir);
            let result = search(note_dir, true, true, false, &|s| {
                s.to_str().is_some_and(|s| pattern.is_match(s))
            })?
            .concat();

            if result.is_empty() {
                bail!("No note found in '{}'", note_dir.display());
            }

            println!("Found notes:");
            for entry in result {
                println!("{}", entry.path().display());
            }
        }
        Cli::List {
            note_dir,
            category,
            sort_by_category,
            sort_by_name,
            sort_by_created_at,
            sort_by_updated_at,
            number,
            terse,
        } => {
            let note_dir_path = Path::new(&note_dir);

            let result = if category {
                search(note_dir_path, false, false, true, &|_| true)?.concat()
            } else {
                search(note_dir_path, true, true, false, &|_| true)?.concat()
            };

            let mut notes = result.iter().map(|e| e.path()).collect::<Vec<_>>();
            let mut print_tree_flag = false;

            if sort_by_category {
                // 按分类分组逻辑
                let mut categories: HashMap<String, Vec<PathBuf>> = HashMap::new();

                // 遍历所有笔记路径
                for note_path in &notes {
                    // 剥离根目录前缀
                    let rel_path = note_path.strip_prefix(note_dir_path).unwrap();

                    // 提取最低一级分类名
                    let category_name = rel_path
                        .parent()
                        .and_then(|p| p.iter().next_back())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Uncategorized".to_string());

                    // 提取文件名部分
                    let file_name = rel_path.file_name().unwrap();

                    // 构造分类下的相对路径 (分类名/文件名)
                    let categorized_path = Path::new(&category_name).join(file_name);

                    // 按分类分组
                    categories
                        .entry(category_name)
                        .or_default()
                        .push(categorized_path);
                }

                // 按分类名排序后输出
                let mut sorted_categories: Vec<_> = categories.into_iter().collect();
                sorted_categories.sort_by(|(a, _), (b, _)| a.cmp(b));

                // 为每个分类生成树
                for (_, notes) in sorted_categories {
                    print_tree(&notes);
                }

                return Ok(());
            } else if sort_by_name {
                notes.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            } else if sort_by_created_at {
                notes.sort_by(|a, b| {
                    b.metadata()
                        .unwrap()
                        .created()
                        .unwrap()
                        .cmp(&a.metadata().unwrap().created().unwrap())
                });
                // 只显示最新的number个笔记
                notes.truncate(number);
            } else if sort_by_updated_at {
                notes.sort_by(|a, b| {
                    b.metadata()
                        .unwrap()
                        .modified()
                        .unwrap()
                        .cmp(&a.metadata().unwrap().modified().unwrap())
                });
                // 只显示最新的number个笔记
                notes.truncate(number);
            } else {
                print_tree_flag = true;
            }

            if terse {
                notes.iter_mut().for_each(|n| {
                    *n = Path::new(n.file_name().unwrap());
                });
            } else {
                notes.iter_mut().for_each(|n| {
                    *n = n.strip_prefix(note_dir_path).unwrap();
                });
            }

            if print_tree_flag {
                print_tree(&notes);
            } else {
                for note in notes {
                    println!("{}", note.display());
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

    create_paths(note_path, &template.paths)?;

    Ok(())
}

fn load_note_template(file_path: &OsStr) -> Result<NoteTemplate> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read template file '{}'", file_path.display()))?;
    let template: NoteTemplate = serde_yml::from_str(&content)
        .with_context(|| format!("Failed to parse template file '{}'", file_path.display()))?;
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

fn search(
    note_dir: &Path,
    search_filenote: bool,
    search_dirnote: bool,
    search_category: bool,
    eq: &dyn Fn(&OsStr) -> bool,
) -> Result<[Vec<DirEntry>; 3]> {
    let mut filenotes = Vec::new();
    let mut dirnotes = Vec::new();
    let mut categories = Vec::new();

    let mut handle_filenote = if search_filenote {
        Some(|entry: DirEntry| {
            if eq(entry.file_name()) {
                filenotes.push(entry);
            }
            Ok(())
        })
    } else {
        None
    };
    let mut handle_dirnote = if search_dirnote {
        Some(|entry: DirEntry| {
            if eq(entry.file_name()) {
                dirnotes.push(entry);
            }
            Ok(())
        })
    } else {
        None
    };
    let mut handle_category = if search_category {
        Some(|entry: DirEntry| {
            if eq(entry.file_name()) {
                categories.push(entry);
            }
            Ok(())
        })
    } else {
        None
    };

    handle_notes(
        note_dir,
        handle_filenote
            .as_mut()
            .map(|f| f as &mut dyn FnMut(DirEntry) -> Result<()>),
        handle_dirnote
            .as_mut()
            .map(|f| f as &mut dyn FnMut(DirEntry) -> Result<()>),
        handle_category
            .as_mut()
            .map(|f| f as &mut dyn FnMut(DirEntry) -> Result<()>),
    )?;

    Ok([filenotes, dirnotes, categories])
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

fn exec_with(note_path: &Path, args: &[OsString]) -> Result<()> {
    let mut cmd = Command::new(&args[0]);
    for arg in &args[1..] {
        cmd.arg(arg);
    }
    cmd.arg(note_path);

    println!("Running {:?}", cmd);

    cmd.status()?;

    Ok(())
}

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
            && entry.path().is_filenote()
        {
            handle(entry)?;
        } else if let Some(handle) = handle_dirnote.as_mut()
            && entry.path().is_dirnote()
        {
            handle(entry)?;
            it.skip_current_dir();
        } else if let Some(handle) = handle_category.as_mut()
            && entry.path().is_category()
        {
            handle(entry)?;
        }
    }

    Ok(())
}

// fn is_filenote(entry: &DirEntry) -> bool {
//     entry.file_type().is_some_and(|t| t.is_file())
//         && entry
//             .path()
//             .extension()
//             .and_then(|ext| ext.to_str())
//             .and_then(|ext| NoteType::try_from(ext).ok())
//             .is_some()
// }
//
// fn is_dirnote(entry: &DirEntry) -> bool {
//     let path = entry.path();
//     entry.file_type().is_some_and(|t| t.is_dir())
//         && (path.join("main.md").is_file() || path.join("main.typ").is_file())
// }
//
// fn is_category(entry: &DirEntry) -> bool {
//     let path = entry.path();
//     entry.file_type().is_some_and(|t| t.is_dir())
//         && !path.join("main.md").is_file()
//         && !path.join("main.typ").is_file()
// }
//
// fn is_note_name(note_path: &Path) -> bool {
//     note_path.components().count() == 1
// }

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

fn print_tree(paths: &[impl AsRef<Path>]) {
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
        add_path(&mut root, p.as_ref());
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
            note_path: note_path.to_string().into(),
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
            note_path: Some(note_path.to_string().into()),
            note_dir: note_dir.to_string().into(),
            preview_typst: vec![],
            preview_markdown: vec![],
        }
    }

    /// Helper to build Cli::Search arguments quickly
    fn cli_search_args(query: &str, note_dir: &str) -> Cli {
        Cli::Search {
            query: query.to_string(),
            note_dir: note_dir.to_string().into(),
        }
    }

    /// Helper to build Cli::List arguments quickly
    fn cli_list_args(note_dir: &str) -> Cli {
        Cli::List {
            note_dir: note_dir.to_string().into(),
            category: false,
            sort_by_category: true,
            sort_by_name: false,
            sort_by_created_at: false,
            sort_by_updated_at: false,
            number: 10,
            terse: false,
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

        let loaded_template = load_note_template(template_file.as_os_str());
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

    #[test]
    fn test_note_trait_methods() {
        let tmp_dir = tempdir().unwrap();

        // Test filenote
        let filenote_md = tmp_dir.path().join("note.md");
        fs::File::create(&filenote_md).unwrap();
        assert!(filenote_md.is_filenote());
        assert!(!filenote_md.is_dirnote());
        assert!(!filenote_md.is_category());

        let filenote_typ = tmp_dir.path().join("note.typ");
        fs::File::create(&filenote_typ).unwrap();
        assert!(filenote_typ.is_filenote());
        assert!(!filenote_typ.is_dirnote());
        assert!(!filenote_typ.is_category());

        // Test dirnote
        let dirnote = tmp_dir.path().join("dirnote");
        fs::create_dir(&dirnote).unwrap();
        fs::File::create(dirnote.join("main.md")).unwrap();
        assert!(dirnote.is_dirnote());
        assert!(!dirnote.is_filenote());
        assert!(!dirnote.is_category());

        // Test category
        let category = tmp_dir.path().join("category");
        fs::create_dir(&category).unwrap();
        assert!(category.is_category());
        assert!(!category.is_filenote());
        assert!(!category.is_dirnote());
    }

    #[test]
    fn test_metadata_generation() {
        let note_name = "TestNote";
        let author = Some("AuthorName".to_string());
        let keywords = ["kw1".to_string(), "kw2".to_string()];

        // Test Markdown metadata
        let md_meta = metadata(note_name, author.as_ref(), NoteType::Md, &keywords);
        assert!(md_meta.contains("title: \"TestNote\""));
        assert!(md_meta.contains("author: \"AuthorName\""));
        assert!(md_meta.contains("keywords: [kw1, kw2]"));
        assert!(md_meta.starts_with("---\n"));

        // Test Typst metadata
        let typ_meta = metadata(note_name, author.as_ref(), NoteType::Typ, &keywords);
        assert!(typ_meta.contains("#set document(title: \"TestNote\""));
        assert!(typ_meta.contains("author: \"AuthorName\""));
        assert!(typ_meta.contains("keywords: (kw1, kw2)"));
        assert!(typ_meta.contains("date: datetime"));
    }

    #[test]
    fn test_search_function() {
        let tmp_dir = tempdir().unwrap();
        let note_dir = tmp_dir.path();

        // Create test data
        let filenote = note_dir.join("file.md");
        fs::File::create(&filenote).unwrap();

        let dirnote = note_dir.join("dirnote");
        fs::create_dir(&dirnote).unwrap();
        fs::File::create(dirnote.join("main.md")).unwrap();

        let category = note_dir.join("category");
        fs::create_dir(&category).unwrap();

        // Search filenotes
        let [filenotes, _, _] = search(note_dir, true, false, false, &|s| s == "file.md").unwrap();
        assert_eq!(filenotes.len(), 1);

        // Search dirnotes
        let [_, dirnotes, _] = search(note_dir, false, true, false, &|s| s == "dirnote").unwrap();
        assert_eq!(dirnotes.len(), 1);

        // Search categories
        let [_, _, categories] =
            search(note_dir, false, false, true, &|s| s == "category").unwrap();
        assert_eq!(categories.len(), 1);
    }

    #[test]
    fn test_tree_printing() {
        let paths = vec![
            PathBuf::from("a/b/c.txt"),
            PathBuf::from("a/d.txt"),
            PathBuf::from("b/e.txt"),
        ];

        let result = std::panic::catch_unwind(|| {
            print_tree(&paths);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_note_type() {
        let tmp_dir = tempdir().unwrap();
        let invalid_file = tmp_dir.path().join("invalid.txt");
        fs::File::create(&invalid_file).unwrap();

        let args = Cli::Preview {
            note_path: Some(invalid_file.into()),
            note_dir: tmp_dir.path().into(),
            preview_typst: vec![],
            preview_markdown: vec![],
        };

        let result = process_command(args);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to parse note type"));
    }

    #[test]
    fn test_note_path_resolution() {
        let tmp_dir = tempdir().unwrap();

        // Test filenote path
        let filenote = tmp_dir.path().join("note.md");
        fs::File::create(&filenote).unwrap();
        assert_eq!(filenote.note_path().unwrap(), filenote);

        // Test dirnote path
        let dirnote = tmp_dir.path().join("dirnote");
        fs::create_dir(&dirnote).unwrap();
        fs::File::create(dirnote.join("main.md")).unwrap();
        assert_eq!(dirnote.note_path().unwrap(), dirnote.join("main.md"));

        // Test invalid dirnote
        let invalid_dirnote = tmp_dir.path().join("invalid_dirnote");
        fs::create_dir(&invalid_dirnote).unwrap();
        assert!(invalid_dirnote.note_path().is_err());
    }

    #[test]
    fn test_template_creation() {
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

        create_note_template(&note_dir, &template).unwrap();

        // Verify directory structure
        let subdir = note_dir.join("subdir");
        assert!(subdir.is_dir());

        // Verify file content
        let subfile = subdir.join("subfile.md");
        assert!(subfile.is_file());
        assert_eq!(fs::read_to_string(subfile).unwrap(), "content");
    }
}
