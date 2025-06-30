use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use roxmltree::Document;

/// Static instance of `DjVuMessage`, initialized lazily on first use.
static MESSAGE: OnceLock<DjVuMessage> = OnceLock::new();

/// Struct representing the DjVuMessage system, holding a map of message IDs to their texts.
pub struct DjVuMessage {
    messages: HashMap<String, String>,
    programname: Option<String>,
}

impl DjVuMessage {
    /// Creates a new `DjVuMessage` instance by loading messages from XML files.
    fn new() -> Self {
        let messages = load_messages().unwrap_or_default();
        DjVuMessage {
            messages,
            programname: None,
        }
    }

    /// Looks up a message list, processing multiple messages separated by newlines.
    fn lookup(&self, message_list: &str) -> String {
        let mut result = String::new();
        let mut start = 0;
        while start < message_list.len() {
            if message_list.as_bytes()[start] == b'\n' {
                result.push('\n');
                start += 1;
            } else {
                let end = message_list[start..]
                    .find('\n')
                    .map_or(message_list.len(), |i| start + i);
                let single_message = &message_list[start..end];
                result.push_str(&self.lookup_single(single_message));
                start = end;
            }
        }
        result
    }

    /// Processes a single message, handling message IDs and parameters.
    fn lookup_single(&self, single_message: &str) -> String {
        // Check for control character (CTRL-C, '\003') if required by original behavior
        let mut parts = Vec::new();
        let mut separators = Vec::new();
        let mut start = 0;

        while start < single_message.len() {
            let next_tab = single_message[start..].find('\t').map(|i| start + i);
            let next_vtab = single_message[start..].find('\x0b').map(|i| start + i);
            let next = match (next_tab, next_vtab) {
                (Some(t), Some(v)) if t < v => Some((t, '\t')),
                (Some(t), _) => Some((t, '\t')),
                (_, Some(v)) => Some((v, '\x0b')),
                (None, None) => None,
            };

            if let Some((pos, sep)) = next {
                if pos > start {
                    parts.push(&single_message[start..pos]);
                }
                separators.push(sep);
                start = pos + 1;
            } else {
                if start < single_message.len() {
                    parts.push(&single_message[start..]);
                }
                break;
            }
        }

        if parts.is_empty() {
            return String::new();
        }

        let message_id = parts[0].to_string();
        let mut msg_text = self.get_message_text(&message_id);

        if msg_text.is_empty() {
            msg_text = format!("** Unrecognized DjVu Message: {}", message_id);
            for (i, part) in parts.iter().enumerate().skip(1) {
                let param = if i - 1 < separators.len() && separators[i - 1] == '\x0b' {
                    self.lookup_single(part)
                } else {
                    part.to_string()
                };
                msg_text.push_str(&format!("\n\t** Parameter: {}", param));
            }
        } else {
            for (i, part) in parts.iter().enumerate().skip(1) {
                let arg = if i - 1 < separators.len() && separators[i - 1] == '\x0b' {
                    self.lookup_single(part)
                } else {
                    part.to_string()
                };
                self.insert_arg(&mut msg_text, i, &arg);
            }
        }
        msg_text
    }

    /// Retrieves the message text for a given message ID from the map.
    fn get_message_text(&self, message_id: &str) -> String {
        self.messages.get(message_id).cloned().unwrap_or_default()
    }

    /// Inserts an argument into the message text, replacing placeholders like `%n!s!`.
    fn insert_arg(&self, message: &mut String, arg_id: usize, arg: &str) {
        let target = format!("%{}!", arg_id);
        let re = Regex::new(&format!(r"%{}!([a-zA-Z0-9.+-]*)!", arg_id)).unwrap();
        while let Some(mat) = re.find(message) {
            let format_str = mat.as_str().trim_start_matches(&target).trim_end_matches('!');
            let formatted = self.format_arg(arg, format_str);
            *message = message.replace(mat.as_str(), &formatted);
        }
    }

    /// Formats an argument based on the format specifier (e.g., `s`, `d`, `f`).
    fn format_arg(&self, arg: &str, format: &str) -> String {
        if format.is_empty() {
            return arg.to_string();
        }
        match format.chars().last().unwrap_or('s') {
            's' => arg.to_string(),
            'd' | 'i' => arg
                .parse::<i32>()
                .map_or(arg.to_string(), |v| format!("{}", v)),
            'u' => arg
                .parse::<u32>()
                .map_or(arg.to_string(), |v| format!("{}", v)),
            'x' => arg
                .parse::<u32>()
                .map_or(arg.to_string(), |v| format!("{:x}", v)),
            'X' => arg
                .parse::<u32>()
                .map_or(arg.to_string(), |v| format!("{:X}", v)),
            'f' => arg
                .parse::<f64>()
                .map_or(arg.to_string(), |v| format!("{}", v)),
            _ => arg.to_string(),
        }
    }

    /// Sets the program name, which may influence path resolution (optional).
    fn set_programname(&mut self, name: &str) {
        self.programname = Some(name.to_string());
        // Optionally reload messages if programname affects paths
    }
}

/// Loads messages from XML files found in various profile paths.
fn load_messages() -> Result<HashMap<String, String>, Box<dyn Error>> {
    let paths = get_profile_paths();
    let mut included = HashSet::new();
    let mut messages = HashMap::new();

    for path in paths {
        let url = path.join("messages.xml");
        if url.is_file() {
            let file_messages = load_file_messages(&url, &mut included)?;
            for (k, v) in file_messages {
                messages.entry(k).or_insert(v);
            }
        }
    }
    Ok(messages)
}

/// Recursively loads messages from an XML file, handling `<INCLUDE>` tags.
fn load_file_messages(
    url: &Path,
    included: &mut HashSet<String>,
) -> Result<HashMap<String, String>, Box<dyn Error>> {
    let url_str = url.to_string_lossy().to_string();
    if included.contains(&url_str) {
        return Ok(HashMap::new());
    }
    included.insert(url_str);

    let xml = fs::read_to_string(url)?;
    let doc = Document::parse(&xml)?;
    let mut messages = HashMap::new();

    // Handle includes in <HEAD>
    if let Some(head) = doc.root().children().find(|n| n.has_tag_name("HEAD")) {
        for include in head.children().filter(|n| n.has_tag_name("INCLUDE")) {
            if let Some(name) = include.attribute("name") {
                let include_url = url.parent().unwrap().join(name);
                let included_messages = load_file_messages(&include_url, included)?;
                for (k, v) in included_messages {
                    messages.entry(k).or_insert(v);
                }
            }
        }
    }

    // Extract messages from <BODY>
    if let Some(body) = doc.root().children().find(|n| n.has_tag_name("BODY")) {
        for message in body.children().filter(|n| n.has_tag_name("MESSAGE")) {
            if let Some(name) = message.attribute("name") {
                let text = message
                    .attribute("value")
                    .map_or_else(|| message.text().unwrap_or("").to_string(), String::from);
                messages.insert(name.to_string(), text);
            }
        }
    }
    Ok(messages)
}

/// Determines the profile paths to search for XML files, considering locale and system directories.
fn get_profile_paths() -> Vec<PathBuf> {
    let base_paths = get_base_paths();
    let current_locale = get_current_locale();
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    // Check for language-specific paths
    for base in &base_paths {
        let lang_file = base.join("languages.xml");
        if lang_file.is_file() {
            if let Ok(xml) = fs::read_to_string(&lang_file) {
                if let Ok(doc) = Document::parse(&xml) {
                    if let Some(body) = doc.root().first_child() {
                        for language in body.children().filter(|n| n.has_tag_name("LANGUAGE")) {
                            if let Some(locale) = language.attribute("locale") {
                                if locale == current_locale {
                                    if let Some(src) = language.attribute("src") {
                                        let lang_path = base.join(src);
                                        if lang_path.is_dir() && !seen.contains(&lang_path) {
                                            seen.insert(lang_path.clone());
                                            paths.push(lang_path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Append base paths
    for base in base_paths {
        if !seen.contains(&base) {
            seen.insert(base.clone());
            paths.push(base);
        }
    }
    paths
}

/// Collects base paths where XML files might be located.
fn get_base_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Environment variable DJVU_CONFIG_DIR
    if let Ok(dir) = env::var("DJVU_CONFIG_DIR") {
        paths.push(PathBuf::from(dir));
    }

    // Executable directory and related paths
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            paths.push(exe_dir.join("share/djvu/osi"));
            paths.push(exe_dir.join("profiles"));
            if let Some(parent) = exe_dir.parent() {
                paths.push(parent.join("share/djvu/osi"));
                paths.push(parent.join("profiles"));
            }
        }
    }

    // Home directory
    if let Ok(home) = env::var("HOME") {
        paths.push(PathBuf::from(home).join(".DjVu"));
    }

    // System-wide directories
    #[cfg(unix)]
    paths.push(PathBuf::from("/etc/DjVu/"));
    #[cfg(windows)]
    paths.push(PathBuf::from("C:/Program Files/DjVu/"));

    paths
}

/// Retrieves the current locale from environment variables.
fn get_current_locale() -> String {
    env::var("LANGUAGE")
        .or_else(|_| env::var("LC_MESSAGES"))
        .or_else(|_| env::var("LC_ALL"))
        .or_else(|_| env::var("LANG"))
        .unwrap_or_else(|_| "C".to_string())
}

/// Public API: Looks up a message list and returns the formatted result.
pub fn lookup_utf8(message_list: &str) -> String {
    MESSAGE.get_or_init(|| DjVuMessage::new()).lookup(message_list)
}

/// Public API: Prints an error message to stderr.
pub fn perror(message_list: &str) {
    eprintln!("{}", lookup_utf8(message_list));
}

/// Public API: Sets the program name (optional functionality).
pub fn set_programname(name: &str) {
    MESSAGE.get_or_init(|| DjVuMessage::new()).set_programname(name);
}