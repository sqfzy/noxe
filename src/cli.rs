use std::ffi::OsString;

use clap::{
    Parser, ValueEnum, builder::NonEmptyStringValueParser, crate_authors, crate_description,
    crate_name, crate_version,
};

#[derive(Parser, Debug)]
#[command(
    name = crate_name!(),
    author = crate_authors!(),
    version = crate_version!(),
    about = crate_description!()
)]
pub enum Cli {
    #[command(about = "Create a new note")]
    New {
        /// The path of the note. If the note path includes an extension (e.g., .md or .typ), the note type will be
        /// automatically inferred and the note will be created as a single file.
        #[arg(value_parser = NonEmptyStringValueParser::new())]
        note_path: String,

        /// The author of the note
        #[arg(short = 'a', long, env = "NOXE_AUTHOR")]
        note_author: Option<String>,

        /// Specify keywords for the note (comma-separated)
        #[arg(short = 'k', long, value_delimiter = ',')]
        note_keywords: Vec<String>,

        /// Specify the note type (md|typ). Default is 'typ'
        #[arg(short = 't', long, default_value_t, value_enum, env = "NOXE_TYPE")]
        note_type: NoteType,

        #[arg(short = 's', long, default_value = "false")]
        single_file: bool,

        #[arg(short = 'S', long, env = "NOXE_TEMPLATE")]
        note_template: Option<String>,

        #[arg(short = 'm', long, default_value = "true")]
        note_with_metadata: bool,
    },

    #[command(about = "Preview the note")]
    Preview {
        /// The path or name of the note. When it is a name, the note will be searched in the note directory.
        /// When it is a path, the note will be found in the specified path.
        #[arg(value_parser = NonEmptyStringValueParser::new())]
        note_path: String,

        /// The directory where the notes are stored
        #[arg(short = 'd', long, default_value = ".", env = "NOXE_DIR")]
        note_dir: String,

        /// Custom typst preview command. The note path will automatically be appended to the command.
        /// eg. `tinymist preview`
        #[arg(long, value_delimiter = ' ', env = "NOXE_PREVIEW_TYPST")]
        preview_typst: Vec<OsString>,

        /// Custom markdown preview command. The note path will automatically be appended to the command.
        /// eg. `glow`
        #[arg(long, value_delimiter = ' ', env = "NOXE_PREVIEW_MARKDOWN")]
        preview_markdown: Vec<OsString>,
    },

    #[command(about = "Search notes")]
    Search {
        /// The query to search for
        #[arg(value_parser = NonEmptyStringValueParser::new())]
        query: String,

        /// The directory where the notes are stored
        #[arg(short = 'd', long, default_value = ".", env = "NOXE_DIR")]
        note_dir: String,
    },

    #[command(about = "List notes")]
    List {
        /// The directory where the notes are stored
        #[arg(short = 'd', long, default_value = ".", env = "NOXE_DIR")]
        note_dir: String,

        /// List categories
        #[arg(short = 'a', default_value = "false", group = "sort")]
        category: bool,

        /// List notes by category
        #[arg(short = 'c', default_value = "false", group = "sort")]
        sort_by_category: bool,

        /// List notes by name
        #[arg(short = 'n', default_value = "false", group = "sort")]
        sort_by_name: bool,

        /// List notes by created date
        #[arg(short = 'C', default_value = "false", group = "sort")]
        sort_by_created_at: bool,

        /// List notes by updated date
        #[arg(short = 'u', default_value = "false", group = "sort")]
        sort_by_updated_at: bool,

        /// The number of notes to list
        #[arg(short = 'N', long, default_value = "10")]
        number: usize,

        /// Only list notes file name
        #[arg(short = 't', long, default_value = "false")]
        terse: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum NoteType {
    #[default]
    Typ,
    Md,
}

impl std::fmt::Display for NoteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoteType::Typ => write!(f, "typ"),
            NoteType::Md => write!(f, "md"),
        }
    }
}

impl TryFrom<&str> for NoteType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "typ" => Ok(NoteType::Typ),
            "md" => Ok(NoteType::Md),
            _ => Err(format!("Invalid note type: {}", value)),
        }
    }
}

impl From<NoteType> for &'static str {
    fn from(val: NoteType) -> Self {
        match val {
            NoteType::Typ => "typ",
            NoteType::Md => "md",
        }
    }
}
