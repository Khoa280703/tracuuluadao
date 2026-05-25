use std::collections::HashSet;
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

    /// Returns up to `count` distinct proxies, excluding given source files,
    /// preferring different source files among the remaining.
    pub fn pick_distinct_excluding(
        &self,
        count: usize,
        excluded_sources: &HashSet<String>,
    ) -> Vec<&ProxyConfig> {
        if excluded_sources.is_empty() {
            return self.pick_distinct(count);
        }
        let filtered: Vec<&ProxyConfig> = self
            .proxies
            .iter()
            .filter(|p| !excluded_sources.contains(&p.source_file))
            .collect();
        Self::pick_distinct_from(&filtered, count)
    }

    /// Returns up to `count` distinct proxies, preferring different source files.
    pub fn pick_distinct(&self, count: usize) -> Vec<&ProxyConfig> {
        let refs: Vec<&ProxyConfig> = self.proxies.iter().collect();
        Self::pick_distinct_from(&refs, count)
    }

    fn pick_distinct_from<'a>(proxies: &[&'a ProxyConfig], count: usize) -> Vec<&'a ProxyConfig> {
        if proxies.is_empty() || count == 0 {
            return Vec::new();
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos() as usize)
            .unwrap_or_default();

        let mut result: Vec<&ProxyConfig> = Vec::with_capacity(count);
        let mut seen_sources = HashSet::new();

        let start = nanos % proxies.len();
        for offset in 0..proxies.len() {
            if result.len() >= count {
                break;
            }
            let proxy = proxies[(start + offset) % proxies.len()];
            if seen_sources.insert(&proxy.source_file) {
                result.push(proxy);
            }
        }

        for offset in 0..proxies.len() {
            if result.len() >= count {
                break;
            }
            let proxy = proxies[(start + offset) % proxies.len()];
            if !result.iter().any(|p| std::ptr::eq(*p, proxy)) {
                result.push(proxy);
            }
        }

        result
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
    use super::{ProxyConfig, ProxyPool, parse_proxy_line};

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

    #[test]
    fn pick_distinct_returns_unique_proxies_from_different_sources() {
        let pool = ProxyPool {
            proxies: vec![
                ProxyConfig {
                    url: "http://a:1".into(),
                    source_file: "file_a".into(),
                    rotation_seed: 0,
                },
                ProxyConfig {
                    url: "http://b:2".into(),
                    source_file: "file_a".into(),
                    rotation_seed: 1,
                },
                ProxyConfig {
                    url: "http://c:3".into(),
                    source_file: "file_b".into(),
                    rotation_seed: 0,
                },
                ProxyConfig {
                    url: "http://d:4".into(),
                    source_file: "file_c".into(),
                    rotation_seed: 0,
                },
            ],
        };
        let picks = pool.pick_distinct(3);
        assert_eq!(picks.len(), 3);
        // All picks should be distinct pointers
        for i in 0..picks.len() {
            for j in (i + 1)..picks.len() {
                assert!(!std::ptr::eq(picks[i], picks[j]));
            }
        }
    }

    #[test]
    fn pick_distinct_on_empty_pool_returns_empty() {
        let pool = ProxyPool::default();
        assert!(pool.pick_distinct(3).is_empty());
    }
}
