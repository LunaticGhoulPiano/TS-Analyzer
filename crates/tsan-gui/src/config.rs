use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub theme: String,
    pub opacity: u8,
    pub player_backend: String,
    pub recent_files: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "Light".into(),
            opacity: 195,
            player_backend: "d3d12".into(),
            recent_files: Vec::new(),
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TSAN_CONFIG_PATH") {
        return Some(path.into());
    }
    #[cfg(target_os = "windows")]
    let directory = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let directory =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    directory.map(|p| p.join("TS-Analyzer").join("tsan-config.toml"))
}

pub fn load(path: &Path) -> Result<Settings, String> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut settings = Settings::default();
            let legacy = path.with_file_name("recent-files.txt");
            match fs::read_to_string(legacy) {
                Ok(text) => {
                    settings.recent_files = text
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(PathBuf::from)
                        .take(12)
                        .collect()
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("Could not read legacy recent files: {e}")),
            }
            Ok(settings)
        }
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

fn quoted(text: &str) -> String {
    let mut result = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(result, "\\u{:04X}", c as u32);
            }
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

pub fn encode(settings: &Settings) -> String {
    let mut text = format!(
        "# TS Analyzer settings. UTF-8; paths may use TOML basic or literal strings.\n\
         config_version = 1\ntheme = {}\ntransparent_opacity = {}\nplayer_backend = {}\nrecent_files = [\n",
        quoted(&settings.theme),
        settings.opacity,
        quoted(&settings.player_backend)
    );
    for path in settings.recent_files.iter().take(12) {
        text.push_str(&format!("  {},\n", quoted(&path.to_string_lossy())));
    }
    text.push_str("]\n");
    text
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    use std::io::Write as _;
    let parent = path.parent().ok_or("Config path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(encode(settings).as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    // std::fs::rename replaces the destination atomically, including on Windows.
    fs::rename(&temp, path).map_err(|e| format!("Could not replace {}: {e}", path.display()))
}

// Small, strict reader for this versioned flat settings schema. Unknown keys or unsupported
// TOML constructs fail without overwriting the user's file.
struct Reader {
    chars: Vec<char>,
    position: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }
    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.position += 1;
        Some(c)
    }
    fn space(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.position += 1;
            }
            if self.peek() != Some('#') {
                break;
            }
            while self.peek().is_some_and(|c| c != '\n') {
                self.position += 1;
            }
        }
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        self.space();
        if self.next() == Some(c) {
            Ok(())
        } else {
            Err(format!("Expected '{c}' in config"))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.space();
        let quote = self.next().ok_or("Missing string")?;
        if !matches!(quote, '"' | '\'') {
            return Err("Expected a quoted TOML string".into());
        }
        let mut result = String::new();
        while let Some(c) = self.next() {
            if c == quote {
                return Ok(result);
            }
            if c.is_control() {
                return Err("Unescaped control character in config".into());
            }
            if c == '\\' && quote == '"' {
                let escape = self.next().ok_or("Incomplete escape")?;
                result.push(match escape {
                    '"' => '"',
                    '\\' => '\\',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'u' | 'U' => {
                        let mut value = 0u32;
                        for _ in 0..if escape == 'u' { 4 } else { 8 } {
                            let digit = self
                                .next()
                                .and_then(|c| c.to_digit(16))
                                .ok_or("Invalid Unicode escape")?;
                            value = value
                                .checked_mul(16)
                                .and_then(|v| v.checked_add(digit))
                                .ok_or("Unicode escape overflow")?;
                        }
                        char::from_u32(value).ok_or("Invalid Unicode scalar")?
                    }
                    _ => return Err(format!("Unsupported escape: \\{escape}")),
                });
            } else {
                result.push(c);
            }
        }
        Err("Unterminated string".into())
    }
    fn integer(&mut self) -> Result<u32, String> {
        self.space();
        let start = self.position;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.position += 1;
        }
        self.chars[start..self.position]
            .iter()
            .collect::<String>()
            .parse()
            .map_err(|_| "Expected a nonnegative integer".into())
    }
    fn strings(&mut self) -> Result<Vec<PathBuf>, String> {
        self.expect('[')?;
        let mut values = Vec::new();
        loop {
            self.space();
            if self.peek() == Some(']') {
                self.next();
                return Ok(values);
            }
            values.push(PathBuf::from(self.string()?));
            self.space();
            match self.next() {
                Some(']') => return Ok(values),
                Some(',') => {}
                _ => return Err("Expected ',' or ']' in recent_files".into()),
            }
        }
    }
}

pub fn parse(text: &str) -> Result<Settings, String> {
    if text.len() > 1_048_576 {
        return Err("Config exceeds 1 MiB".into());
    }
    let mut reader = Reader {
        chars: text.trim_start_matches('\u{feff}').chars().collect(),
        position: 0,
    };
    let mut settings = Settings::default();
    let mut seen = std::collections::BTreeSet::new();
    loop {
        reader.space();
        if reader.peek().is_none() {
            break;
        }
        let start = reader.position;
        while reader
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            reader.position += 1;
        }
        let key = reader.chars[start..reader.position]
            .iter()
            .collect::<String>();
        if !seen.insert(key.clone()) {
            return Err(format!("Duplicate config key: {key}"));
        }
        reader.expect('=')?;
        match key.as_str() {
            "config_version" if reader.integer()? == 1 => {}
            "theme" => settings.theme = reader.string()?,
            "transparent_opacity" => {
                settings.opacity = u8::try_from(reader.integer()?)
                    .map_err(|_| "transparent_opacity must be 0..255")?
            }
            "player_backend" => settings.player_backend = reader.string()?,
            "recent_files" => settings.recent_files = reader.strings()?,
            _ => return Err(format!("Unsupported config key/version: {key}")),
        }
        while reader
            .peek()
            .is_some_and(|c| c == ' ' || c == '\t' || c == '\r')
        {
            reader.position += 1;
        }
        if reader.peek() == Some('#') {
            while reader.peek().is_some_and(|c| c != '\n') {
                reader.position += 1;
            }
        }
        if reader.peek().is_some_and(|c| c != '\n') {
            return Err("Expected end of config line".into());
        }
    }
    if !matches!(
        settings.theme.as_str(),
        "System" | "Dark" | "Light" | "Transparent" | "Liquid Glass"
    ) {
        return Err("Unknown theme".into());
    }
    if !matches!(settings.player_backend.as_str(), "d3d11" | "d3d12") {
        return Err("Unknown player_backend".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    settings
        .recent_files
        .retain(|p| !p.as_os_str().is_empty() && unique.insert(p.clone()));
    settings.recent_files.truncate(12);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_theme_round_trip_without_losing_escapes() -> Result<(), String> {
        let settings = Settings {
            theme: "Dark".into(),
            recent_files: vec![
                PathBuf::from(r"C:\錄影\#1\video.ts"),
                PathBuf::from("name\"with\ncharacters.ts"),
            ],
            ..Settings::default()
        };
        assert_eq!(parse(&encode(&settings))?, settings);
        let literal = parse("theme = 'Light' # comment\nrecent_files = [\n 'C:\\test.ts',\n]\n")?;
        assert_eq!(literal.recent_files, vec![PathBuf::from(r"C:\test.ts")]);
        assert!(parse("theme = 'Dark'\ntheme = 'Light'\n").is_err());
        assert!(parse("config_version = 2\n").is_err());
        assert!(parse("transparent_opacity = 300\n").is_err());
        Ok(())
    }
    #[test]
    fn legacy_migration_keeps_source_and_atomic_save_replaces_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::temp_dir().join(format!("tsan-config-test-{}", std::process::id()));
        fs::create_dir_all(&directory)?;
        let path = directory.join("tsan-config.toml");
        fs::write(directory.join("recent-files.txt"), "C:\\sample.ts\n")?;
        let mut settings = load(&path)?;
        assert_eq!(settings.recent_files.len(), 1);
        save(&path, &settings)?;
        settings.theme = "Dark".into();
        save(&path, &settings)?;
        assert_eq!(load(&path)?, settings);
        assert!(directory.join("recent-files.txt").exists());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
