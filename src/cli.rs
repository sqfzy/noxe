use clap::{Parser, ValueEnum, builder::NonEmptyStringValueParser};

#[derive(Parser, Debug)]
#[command(
    author = "sqfzy",
    version = "0.1",
    about = "Create a note with specified options"
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
        #[arg(short = 'k', long)]
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
