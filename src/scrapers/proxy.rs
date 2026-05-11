use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyConfig {
    pub url: String,
    pub source_file: String,
    pub rotation_seed: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ProxyPool {
    proxies: Vec<ProxyConfig>,
}

impl ProxyPool {
    pub fn load_from_dir(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut files: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect();
        files.sort();

        let mut proxies = Vec::new();
        for file in files {
            let source_file = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_owned();
            let contents = fs::read_to_string(&file)?;
            for (index, line) in contents.lines().enumerate() {
                if let Some(url) = parse_proxy_line(line) {
                    proxies.push(ProxyConfig {
                        url,
                        source_file: source_file.clone(),
                        rotation_seed: index,
                    });
                }
            }
        }

        Ok(Self { proxies })
    }

    pub fn is_empty(&self) -> bool {
        self.proxies.is_empty()
    }

    pub fn pick(&self) -> Option<&ProxyConfig> {
        if self.proxies.is_empty() {
            return None;
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos() as usize)
            .unwrap_or_default();
        self.proxies.get(nanos % self.proxies.len())
    }
}

fn parse_proxy_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if line.starts_with("http://") || line.starts_with("https://") || line.starts_with("socks5h://")
    {
        return Some(line.to_owned());
    }

    let parts: Vec<&str> = line.split(':').collect();
    match parts.as_slice() {
        [host, port] => Some(format!("http://{host}:{port}")),
        [host, port, user, pass] => Some(format!("http://{user}:{pass}@{host}:{port}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_proxy_line;

    #[test]
    fn parses_proxy_formats() {
        assert_eq!(
            parse_proxy_line("1.2.3.4:80:user:pass").as_deref(),
            Some("http://user:pass@1.2.3.4:80")
        );
        assert_eq!(
            parse_proxy_line("1.2.3.4:80").as_deref(),
            Some("http://1.2.3.4:80")
        );
        assert_eq!(
            parse_proxy_line("socks5h://u:p@1.2.3.4:1080").as_deref(),
            Some("socks5h://u:p@1.2.3.4:1080")
        );
    }
}
