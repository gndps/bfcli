use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

const BFCLI_DIR: &str = ".bfcli";
const SRC_FILES_DIR: &str = "src_files";
const BFLIST_FILE: &str = ".bflist";
const CONFIG_FILE: &str = "config.json";

const DEFAULT_CONFIG: &str = r#"{
  "extensions": ["", ".sh"]
}
"#;

const HELP_TEXT: &str = r#"bfcli — shell file sourcing manager

USAGE:
    bfcli <command>

COMMANDS:
    init      Create ~/.bfcli/ with default config and empty src_files/ dir
    source    Update .bflist and print source commands to stdout
    update    Scan src_files/, write .bflist, print "Updated: N files" to stderr
    files     List all files that will be sourced (one per line)
    config    Open ~/.bfcli/config.json in $EDITOR (fallback: nano)
    help      Print this help message

SETUP:
    1. Run: bfcli init
    2. Add to ~/.bash_profile:
         [ -f ~/.bfcli/.bflist ] && source ~/.bfcli/.bflist
    3. Place shell files in ~/.bfcli/src_files/
    4. Run: bfcli update

CONFIG (~/.bfcli/config.json):
    {
      "extensions": ["", ".sh"]
    }
    ""   = files with no extension
    ".sh" = files with .sh extension
    Hidden files (starting with '.') are always skipped.
"#;

fn bfcli_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| {
        eprintln!("Error: $HOME is not set");
        process::exit(1);
    });
    PathBuf::from(home).join(BFCLI_DIR)
}

/// Parse extensions from config JSON.
/// Expected format: { "extensions": ["", ".sh"] }
/// Uses manual parsing — no external deps.
fn parse_extensions(json: &str) -> Vec<String> {
    // Find the array value for "extensions"
    let key = "\"extensions\"";
    let start = match json.find(key) {
        Some(i) => i,
        None => return vec![String::new(), ".sh".to_string()],
    };
    let after_key = &json[start + key.len()..];

    // Find opening bracket
    let bracket_start = match after_key.find('[') {
        Some(i) => i,
        None => return vec![String::new(), ".sh".to_string()],
    };
    let after_bracket = &after_key[bracket_start + 1..];

    // Find closing bracket
    let bracket_end = match after_bracket.find(']') {
        Some(i) => i,
        None => return vec![String::new(), ".sh".to_string()],
    };
    let array_contents = &after_bracket[..bracket_end];

    // Extract quoted strings
    let mut extensions = Vec::new();
    let mut chars = array_contents.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut s = String::new();
            for inner in chars.by_ref() {
                if inner == '"' {
                    break;
                }
                s.push(inner);
            }
            extensions.push(s);
        }
    }

    if extensions.is_empty() {
        vec![String::new(), ".sh".to_string()]
    } else {
        extensions
    }
}

fn load_extensions(dir: &Path) -> Vec<String> {
    let config_path = dir.join(CONFIG_FILE);
    match fs::read_to_string(&config_path) {
        Ok(contents) => parse_extensions(&contents),
        Err(_) => vec![String::new(), ".sh".to_string()],
    }
}

/// Determine the extension of a filename.
/// Returns None if the file is hidden (starts with '.').
/// Returns Some("") if no dot in the name (e.g. "foo").
/// Returns Some(".sh") for "foo.sh".
/// A file like ".hidden" (starts with dot, no second dot) is hidden -> returns None.
fn file_extension(name: &str) -> Option<&str> {
    if name.starts_with('.') {
        return None; // hidden file, always skip
    }
    match name.rfind('.') {
        Some(pos) => Some(&name[pos..]),
        None => Some(""),
    }
}

/// Recursively collect files under `dir`, sorted alphabetically at each level.
/// Only include files whose extension matches one of `extensions`.
fn collect_files(dir: &Path, extensions: &[String]) -> io::Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if file_type.is_dir() {
            // Skip hidden directories
            if name_str.starts_with('.') {
                continue;
            }
            let mut sub = collect_files(&path, extensions)?;
            results.append(&mut sub);
        } else if file_type.is_file() {
            match file_extension(&name_str) {
                None => continue, // hidden file
                Some(ext) => {
                    if extensions.iter().any(|e| e == ext) {
                        results.push(path);
                    }
                }
            }
        }
    }

    Ok(results)
}

fn cmd_init() {
    let dir = bfcli_dir();

    if dir.exists() {
        eprintln!(
            "Error: {} already exists. Remove it first if you want to reinitialize.",
            dir.display()
        );
        process::exit(1);
    }

    let src_dir = dir.join(SRC_FILES_DIR);
    fs::create_dir_all(&src_dir).unwrap_or_else(|e| {
        eprintln!("Error creating {}: {}", src_dir.display(), e);
        process::exit(1);
    });

    let config_path = dir.join(CONFIG_FILE);
    fs::write(&config_path, DEFAULT_CONFIG).unwrap_or_else(|e| {
        eprintln!("Error writing {}: {}", config_path.display(), e);
        process::exit(1);
    });

    println!("Initialized ~/.bfcli/");
    println!("  Config: {}", config_path.display());
    println!("  Source files dir: {}", src_dir.display());
    println!();
    println!("Next steps:");
    println!("  1. Place shell files in {}", src_dir.display());
    println!("  2. Run: bfcli update");
    println!("  3. Add to ~/.bash_profile:");
    println!("       [ -f ~/.bfcli/.bflist ] && source ~/.bfcli/.bflist");
}

fn do_update(dir: &Path) -> Vec<PathBuf> {
    let src_dir = dir.join(SRC_FILES_DIR);
    if !src_dir.exists() {
        eprintln!("Error: {} does not exist. Run 'bfcli init' first.", src_dir.display());
        process::exit(1);
    }

    let extensions = load_extensions(dir);
    let files = collect_files(&src_dir, &extensions).unwrap_or_else(|e| {
        eprintln!("Error scanning {}: {}", src_dir.display(), e);
        process::exit(1);
    });

    let bflist_path = dir.join(BFLIST_FILE);
    let mut content = String::from("# Generated by bfcli - do not edit manually\n");
    for f in &files {
        content.push_str(&format!("source {}\n", f.display()));
    }

    fs::write(&bflist_path, &content).unwrap_or_else(|e| {
        eprintln!("Error writing {}: {}", bflist_path.display(), e);
        process::exit(1);
    });

    files
}

fn cmd_update() {
    let dir = bfcli_dir();
    let files = do_update(&dir);
    eprintln!("Updated: {} files", files.len());
}

fn cmd_source() {
    let dir = bfcli_dir();
    let files = do_update(&dir);
    eprintln!("Updated: {} files", files.len());
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for f in &files {
        writeln!(out, "source {}", f.display()).unwrap_or_else(|e| {
            eprintln!("Error writing to stdout: {}", e);
            process::exit(1);
        });
    }
}

fn cmd_files() {
    let dir = bfcli_dir();
    let src_dir = dir.join(SRC_FILES_DIR);
    if !src_dir.exists() {
        eprintln!("Error: {} does not exist. Run 'bfcli init' first.", src_dir.display());
        process::exit(1);
    }

    let extensions = load_extensions(&dir);
    let files = collect_files(&src_dir, &extensions).unwrap_or_else(|e| {
        eprintln!("Error scanning {}: {}", src_dir.display(), e);
        process::exit(1);
    });

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for f in &files {
        writeln!(out, "{}", f.display()).unwrap_or_else(|e| {
            eprintln!("Error writing to stdout: {}", e);
            process::exit(1);
        });
    }
}

fn cmd_config() {
    let dir = bfcli_dir();
    let config_path = dir.join(CONFIG_FILE);

    if !config_path.exists() {
        eprintln!("Error: {} does not exist. Run 'bfcli init' first.", config_path.display());
        process::exit(1);
    }

    let editor = env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    let status = process::Command::new(&editor)
        .arg(&config_path)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Error launching editor '{}': {}", editor, e);
            process::exit(1);
        });

    if !status.success() {
        process::exit(status.code().unwrap_or(1));
    }
}

fn main() {
    let command = env::args().nth(1);

    match command.as_deref() {
        Some("init") => cmd_init(),
        Some("update") => cmd_update(),
        Some("source") => cmd_source(),
        Some("files") => cmd_files(),
        Some("config") => cmd_config(),
        Some("help") | Some("--help") | Some("-h") => print!("{}", HELP_TEXT),
        Some(unknown) => {
            eprintln!("Unknown command: {}", unknown);
            eprintln!("Run 'bfcli help' for usage.");
            process::exit(1);
        }
        None => {
            print!("{}", HELP_TEXT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_extension_hidden() {
        assert_eq!(file_extension(".hidden"), None);
        assert_eq!(file_extension(".bashrc"), None);
    }

    #[test]
    fn test_file_extension_no_ext() {
        assert_eq!(file_extension("foo"), Some(""));
        assert_eq!(file_extension("myfile"), Some(""));
    }

    #[test]
    fn test_file_extension_with_ext() {
        assert_eq!(file_extension("foo.sh"), Some(".sh"));
        assert_eq!(file_extension("foo.bash"), Some(".bash"));
        assert_eq!(file_extension("foo.bar.sh"), Some(".sh"));
    }

    #[test]
    fn test_parse_extensions_basic() {
        let json = r#"{ "extensions": ["", ".sh"] }"#;
        let exts = parse_extensions(json);
        assert_eq!(exts, vec!["", ".sh"]);
    }

    #[test]
    fn test_parse_extensions_empty_array() {
        let json = r#"{ "extensions": [] }"#;
        let exts = parse_extensions(json);
        // falls back to defaults when empty
        assert_eq!(exts, vec!["", ".sh"]);
    }

    #[test]
    fn test_parse_extensions_custom() {
        let json = r#"{ "extensions": [".bash", ".zsh"] }"#;
        let exts = parse_extensions(json);
        assert_eq!(exts, vec![".bash", ".zsh"]);
    }
}
