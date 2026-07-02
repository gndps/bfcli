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
    source    Manage/generate the sourceable script (see 'bfcli source' for subcommands)
    appendrc  Manage the line in your shell rc file (see 'bfcli appendrc' for subcommands)
    update    Scan src_files/, write .bflist, print "Updated: N files" to stderr
    files     List all files that will be sourced (one per line)
    config    Open ~/.bfcli/config.json in $EDITOR (fallback: nano)
    help      Print this help message

SETUP:
    1. Run: bfcli init
    2. Run: bfcli appendrc eval
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

const SOURCE_HELP_TEXT: &str = r#"bfcli source — generate/inspect the sourceable script

USAGE:
    bfcli source <subcommand>

SUBCOMMANDS:
    print   Update .bflist and print the sourceable script to stdout (for humans/debugging)
    shell   Print the line to add to your shell rc file (does not modify anything)
    eval    Update .bflist and print the sourceable script, meant to be run inside eval,
            e.g. from your rc file:  eval "$(bfcli source eval)"
            This re-scans src_files/ on every new shell, unlike the static .bflist approach.

NOTE:
    'bfcli source' with no subcommand prints this help. It does NOT source anything itself
    — a subprocess cannot modify its parent shell's environment. Use 'bfcli appendrc eval'
    to wire up automatic sourcing for new shells.
"#;

const APPENDRC_HELP_TEXT: &str = r#"bfcli appendrc — manage the sourcing line in your shell rc file

USAGE:
    bfcli appendrc <subcommand> [--fast]

SUBCOMMANDS:
    shell         Detect the rc file for your OS/shell and print what would be appended
                  (dry run, does not modify anything)
    eval          Detect the rc file, check whether the same path is already referenced
                  (matching "~", "$HOME"/"${HOME}", or an absolute path against it), and
                  append the line only if missing (idempotent)
    eval --fast   Same as 'eval' but skips the existing-content check and appends
                  unconditionally (faster, but can create duplicate lines if run twice)

The rc file is chosen by:
    - $SHELL contains "zsh"           -> ~/.zshrc
    - otherwise, macOS ($OS = Darwin) -> ~/.bash_profile
    - otherwise (e.g. Linux bash)     -> ~/.bashrc
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
    println!("  3. Run: bfcli appendrc eval");
}

/// Escape a path for use inside double-quoted shell strings.
fn shell_escape(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the sourceable script content for a given file list.
/// Each file is sourced through a safe wrapper so that:
///   - missing files print a warning and are skipped (never crash the shell)
///   - files that exit non-zero print a warning but do not propagate the error
///   - the terminal is never killed by a bad sourceable file
/// Note: a file that calls bare `exit` will still close the shell — that
/// cannot be caught without a subshell, which would break sourcing semantics.
/// Files with `exit` calls are scripts and should not be placed in src_files/.
fn build_sourceable_script(files: &[PathBuf]) -> String {
    let mut content = String::from(
        "# Generated by bfcli - do not edit manually\n\
         _bfcli_safe_source() {\n\
           local _bfcli_f=\"$1\"\n\
           if [ ! -f \"$_bfcli_f\" ]; then\n\
             printf '[bfcli] warning: file not found, skipping: %s\\n' \"$_bfcli_f\" >&2\n\
             return 0\n\
           fi\n\
           local _bfcli_opts=\"$-\"\n\
           local _bfcli_pipe=0\n\
           [[ \":${SHELLOPTS}:\" == *:pipefail:* ]] && _bfcli_pipe=1\n\
           set +e +u\n\
           set +o pipefail 2>/dev/null\n\
           source \"$_bfcli_f\"\n\
           local _bfcli_rc=$?\n\
           [[ \"$_bfcli_opts\" == *e* ]] && set -e || set +e\n\
           [[ \"$_bfcli_opts\" == *u* ]] && set -u || set +u\n\
           [ \"$_bfcli_pipe\" -eq 1 ] && set -o pipefail 2>/dev/null\n\
           [ \"$_bfcli_rc\" -ne 0 ] && \\\n\
             printf '[bfcli] warning: error sourcing (exit %s): %s\\n' \"$_bfcli_rc\" \"$_bfcli_f\" >&2\n\
           return 0\n\
         }\n",
    );

    for f in files {
        let escaped = shell_escape(&f.display().to_string());
        content.push_str(&format!("_bfcli_safe_source \"{}\"\n", escaped));
    }
    content.push_str("unset -f _bfcli_safe_source\n");
    content
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

    let content = build_sourceable_script(&files);

    let bflist_path = dir.join(BFLIST_FILE);
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

/// Line to add to a shell rc file to auto-source .bflist in every new shell.
fn bflist_rc_line() -> String {
    "[ -f ~/.bfcli/.bflist ] && source ~/.bfcli/.bflist".to_string()
}

fn print_sourceable_script() {
    let dir = bfcli_dir();
    let files = do_update(&dir);
    eprintln!("Updated: {} files", files.len());
    let content = build_sourceable_script(&files);
    print!("{}", content);
}

fn cmd_source_print() {
    print_sourceable_script();
}

fn cmd_source_eval() {
    print_sourceable_script();
}

fn cmd_source_shell() {
    println!("{}", bflist_rc_line());
}

fn cmd_source(subcommand: Option<&str>) {
    match subcommand {
        Some("print") => cmd_source_print(),
        Some("eval") => cmd_source_eval(),
        Some("shell") => cmd_source_shell(),
        None => print!("{}", SOURCE_HELP_TEXT),
        Some(unknown) => {
            eprintln!("Unknown 'bfcli source' subcommand: {}", unknown);
            eprint!("{}", SOURCE_HELP_TEXT);
            process::exit(1);
        }
    }
}

/// Pick the shell rc file to append the sourcing line to:
///   - $SHELL contains "zsh"     -> ~/.zshrc
///   - otherwise, macOS          -> ~/.bash_profile
///   - otherwise (e.g. Linux)    -> ~/.bashrc
fn detect_rc_file() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| {
        eprintln!("Error: $HOME is not set");
        process::exit(1);
    });
    let shell = env::var("SHELL").unwrap_or_default();
    let rc_name = if shell.contains("zsh") {
        ".zshrc"
    } else if cfg!(target_os = "macos") {
        ".bash_profile"
    } else {
        ".bashrc"
    };
    PathBuf::from(home).join(rc_name)
}

fn cmd_appendrc_shell() {
    let rc_path = detect_rc_file();
    let line = bflist_rc_line();
    println!("Would append to: {}", rc_path.display());
    println!("{}", line);
}

/// Expand "~", "$HOME" and "${HOME}" references to the actual home directory,
/// so path comparisons treat them as equivalent to an absolute path.
fn expand_home_refs(text: &str, home: &str) -> String {
    text.replace("${HOME}", home)
        .replace("$HOME", home)
        .replace('~', home)
}

/// True if `existing` already contains a line referencing the same absolute
/// bflist path as `line`, regardless of whether it was written using "~",
/// "$HOME", or the fully-resolved absolute path.
fn rc_already_has_line(existing: &str, home: &str) -> bool {
    let target = expand_home_refs(&bflist_rc_line(), home);
    existing
        .lines()
        .any(|l| expand_home_refs(l, home).trim() == target)
}

fn append_rc_line(fast: bool) {
    let rc_path = detect_rc_file();
    let line = bflist_rc_line();
    let home = env::var("HOME").unwrap_or_default();

    if !fast {
        let existing = fs::read_to_string(&rc_path).unwrap_or_default();
        if rc_already_has_line(&existing, &home) {
            println!("Already present in {}, skipping.", rc_path.display());
            return;
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rc_path)
        .unwrap_or_else(|e| {
            eprintln!("Error opening {}: {}", rc_path.display(), e);
            process::exit(1);
        });

    writeln!(file, "{}", line).unwrap_or_else(|e| {
        eprintln!("Error writing to {}: {}", rc_path.display(), e);
        process::exit(1);
    });

    println!("Appended to {}", rc_path.display());
}

fn cmd_appendrc_eval(fast: bool) {
    append_rc_line(fast);
}

fn cmd_appendrc(subcommand: Option<&str>, fast: bool) {
    match subcommand {
        Some("shell") => cmd_appendrc_shell(),
        Some("eval") => cmd_appendrc_eval(fast),
        None => print!("{}", APPENDRC_HELP_TEXT),
        Some(unknown) => {
            eprintln!("Unknown 'bfcli appendrc' subcommand: {}", unknown);
            eprint!("{}", APPENDRC_HELP_TEXT);
            process::exit(1);
        }
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
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str());
    let subcommand = args.get(2).map(|s| s.as_str());
    let fast = args.iter().skip(3).any(|a| a == "--fast");

    match command {
        Some("init") => cmd_init(),
        Some("update") => cmd_update(),
        Some("source") => cmd_source(subcommand),
        Some("appendrc") => cmd_appendrc(subcommand, fast),
        Some("files") => cmd_files(),
        Some("config") => cmd_config(),
        Some("help") | Some("--help") | Some("-h") => print!("{}", HELP_TEXT),
        Some("--version") | Some("-V") => println!("bfcli {}", env!("CARGO_PKG_VERSION")),
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

    #[test]
    fn test_rc_already_has_line_exact_match() {
        let home = "/Users/me";
        let existing = "[ -f ~/.bfcli/.bflist ] && source ~/.bfcli/.bflist\n";
        assert!(rc_already_has_line(existing, home));
    }

    #[test]
    fn test_rc_already_has_line_dollar_home() {
        let home = "/Users/me";
        let existing = "[ -f $HOME/.bfcli/.bflist ] && source $HOME/.bfcli/.bflist\n";
        assert!(rc_already_has_line(existing, home));
    }

    #[test]
    fn test_rc_already_has_line_braced_dollar_home() {
        let home = "/Users/me";
        let existing = "[ -f ${HOME}/.bfcli/.bflist ] && source ${HOME}/.bfcli/.bflist\n";
        assert!(rc_already_has_line(existing, home));
    }

    #[test]
    fn test_rc_already_has_line_absolute_path() {
        let home = "/Users/me";
        let existing = "[ -f /Users/me/.bfcli/.bflist ] && source /Users/me/.bfcli/.bflist\n";
        assert!(rc_already_has_line(existing, home));
    }

    #[test]
    fn test_rc_already_has_line_absent() {
        let home = "/Users/me";
        let existing = "export PATH=$PATH:/usr/local/bin\n";
        assert!(!rc_already_has_line(existing, home));
    }
}
