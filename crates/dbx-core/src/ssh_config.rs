use serde::Serialize;

use crate::models::connection::SshTunnelConfig;
use crate::path_utils::expand_tilde;

/// Sentinel values the frontend fills in when the user leaves a field blank
/// (see `normalizeSshTunnel` / `defaultSshTunnel` in ConnectionDialog.vue).
/// There is no way to distinguish "user explicitly typed 22" from "field was
/// left empty and defaulted to 22", so we treat these as "unset" for the
/// purpose of filling in values from `~/.ssh/config`.
const DEFAULT_USER_SENTINEL: &str = "root";
const DEFAULT_PORT_SENTINEL: u16 = 22;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshConfigHostEntry {
    pub alias: String,
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<String>,
}

/// Reads and parses `~/.ssh/config`. Returns an empty list (not an error) if
/// the file does not exist, since that's a normal state for users without an
/// SSH config.
///
/// Only concrete (non-wildcard, non-negated) host patterns are exposed as
/// selectable aliases; wildcard patterns (`*.example.com`, `Host *`) are kept
/// for resolution but intentionally omitted from this list, since they aren't
/// connectable host names on their own.
pub fn list_hosts() -> Result<Vec<SshConfigHostEntry>, String> {
    let path = expand_tilde("~/.ssh/config");
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(list_entries_from_blocks(&parse_host_blocks(&content))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(format!("Failed to read {path}: {err}")),
    }
}

/// Fills in `host`, `user`, `port`, and `key_path` from `~/.ssh/config` `Host`
/// blocks that match `ssh.host`, without overwriting values the user has
/// explicitly set.
///
/// Matching follows OpenSSH semantics: every Host block (including wildcard
/// patterns) is kept in file order; a block applies when `ssh.host` matches at
/// least one positive pattern (`*` any sequence, `?` single char) and no
/// negated (`!`) pattern. Each field is filled independently from the first
/// matching block that provides it (`Host *` is typically a global fallback at
/// the end of the file). `HostName` supports `%h` (original host) and `%%`
/// (literal `%`) token expansion.
///
/// `user`/`port` use the sentinel defaults above to detect "not actually set
/// by the user"; `key_path` is filled only when empty, and a config-supplied
/// key flips `auth_method` to `"key"` when no other credential is present.
pub fn resolve_ssh_tunnel_config(ssh: &SshTunnelConfig) -> SshTunnelConfig {
    let path = expand_tilde("~/.ssh/config");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        // Missing config is a normal no-op state.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ssh.clone(),
        Err(err) => {
            // Unreadable file (permissions, IO) shouldn't break connection
            // attempts; the operator can inspect stderr for the real cause.
            eprintln!("dbx: failed to read ssh config {path}: {err}");
            return ssh.clone();
        }
    };
    let blocks = parse_host_blocks(&content);
    let fields = resolve_host_fields(&blocks, &ssh.host);
    apply_resolved_fields(ssh, &fields)
}

// ---------------------------------------------------------------------------
// Parsing (Host blocks, file order, all patterns kept)
// ---------------------------------------------------------------------------

/// A single host pattern from a `Host` line. `negated` patterns (prefixed with
/// `!`) exclude a host from matching this block even when a positive pattern
/// also matches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostPattern {
    negated: bool,
    text: String,
}

/// A parsed `Host` block: the patterns declared on its `Host` line plus the
/// first value seen for each supported directive within the block. Blocks are
/// kept in file order so resolution can apply first-wins across them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostBlock {
    patterns: Vec<HostPattern>,
    host_name: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
}

/// Per-field resolved values gathered from all matching blocks (first-wins per
/// field). `None` means no matching block set the field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedHostFields {
    host_name: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
}

/// Parses a minimal subset of OpenSSH client config syntax: `Host`, `HostName`,
/// `Port`, `User`, `IdentityFile`. All Host blocks are retained (including
/// wildcard and negated patterns) in file order. Within a block, the first
/// value seen for a given keyword wins, matching OpenSSH's "first-match wins"
/// rule (subsequent same-keyword lines in the same block are ignored).
/// `Include`, `Match`, and other directives are not supported.
fn parse_host_blocks(content: &str) -> Vec<HostBlock> {
    let mut blocks: Vec<HostBlock> = Vec::new();
    let mut current: Option<HostBlock> = None;

    for raw_line in content.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };

        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(block) = current.take() {
                    blocks.push(block);
                }
                current = Some(HostBlock {
                    patterns: value
                        .split_whitespace()
                        .map(|raw| {
                            let (negated, text) = match raw.strip_prefix('!') {
                                Some(rest) => (true, rest.to_string()),
                                None => (false, raw.to_string()),
                            };
                            HostPattern { negated, text }
                        })
                        .collect(),
                    host_name: None,
                    port: None,
                    user: None,
                    identity_file: None,
                });
            }
            "hostname" => set_block_field(&mut current, |b| {
                if b.host_name.is_none() {
                    b.host_name = Some(value.to_string());
                }
            }),
            "port" => set_block_field(&mut current, |b| {
                if b.port.is_none() {
                    if let Ok(port) = value.parse::<u16>() {
                        b.port = Some(port);
                    }
                }
            }),
            "user" => set_block_field(&mut current, |b| {
                if b.user.is_none() {
                    b.user = Some(value.to_string());
                }
            }),
            "identityfile" => set_block_field(&mut current, |b| {
                if b.identity_file.is_none() {
                    b.identity_file = Some(value.to_string());
                }
            }),
            _ => {}
        }
    }

    if let Some(block) = current.take() {
        blocks.push(block);
    }
    blocks
}

/// Applies `f` to the currently-open Host block, if any. Lines appearing
/// before the first `Host` directive are silently dropped (no block to attach
/// to), matching the previous parser's behavior toward free-standing directives.
fn set_block_field(current: &mut Option<HostBlock>, f: impl Fn(&mut HostBlock)) {
    if let Some(block) = current.as_mut() {
        f(block);
    }
}

// ---------------------------------------------------------------------------
// Matching (anchored glob, negation precedence)
// ---------------------------------------------------------------------------

/// Anchored glob match: `*` matches any (possibly empty) sequence of
/// characters, `?` matches exactly one character. Iterates over Unicode chars
/// (not bytes) so non-ASCII host names aren't split mid-codepoint. Case
/// sensitive, matching OpenSSH `match_pattern`.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut star: Option<(usize, usize)> = None; // (pattern index to resume, text index to retry)
    let (mut pi, mut ti) = (0, 0);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            // Remember where to resume the pattern after the star, and that
            // the star currently consumes zero chars of text; backtrack here if
            // a later mismatch occurs.
            star = Some((pi + 1, ti));
            pi += 1;
        } else if let Some((resume_pi, retry_ti)) = star {
            // Make the last star consume one more char of text and retry.
            pi = resume_pi;
            ti = retry_ti + 1;
            star = Some((resume_pi, retry_ti + 1));
        } else {
            return false;
        }
    }

    // Trailing stars match the empty rest of the text.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A block applies when the host matches at least one positive pattern and no
/// negated pattern. A block with only negated patterns never applies
/// (consistent with OpenSSH, which requires a positive match).
fn block_matches(block: &HostBlock, host: &str) -> bool {
    let has_positive = block.patterns.iter().any(|p| !p.negated);
    if !has_positive {
        return false;
    }
    if !block
        .patterns
        .iter()
        .any(|p| !p.negated && wildcard_match(&p.text, host))
    {
        return false;
    }
    if block
        .patterns
        .iter()
        .any(|p| p.negated && wildcard_match(&p.text, host))
    {
        return false;
    }
    true
}

/// Resolves each field independently: the first matching block (in file order)
/// that sets a field provides its value. A later matching block only fills a
/// field left unset by earlier matching blocks — this is how `Host *` at the
/// end of a file acts as a global fallback without overriding specific blocks.
fn resolve_host_fields(blocks: &[HostBlock], host: &str) -> ResolvedHostFields {
    let mut resolved = ResolvedHostFields::default();
    for block in blocks {
        if !block_matches(block, host) {
            continue;
        }
        if resolved.host_name.is_none() {
            resolved.host_name = block.host_name.clone();
        }
        if resolved.port.is_none() {
            resolved.port = block.port;
        }
        if resolved.user.is_none() {
            resolved.user = block.user.clone();
        }
        if resolved.identity_file.is_none() {
            resolved.identity_file = block.identity_file.clone();
        }
    }
    resolved
}

/// Expands `HostName` tokens: `%h` → the original host the user typed, `%%`
/// → a literal `%`. Any other `%X` is passed through literally (including the
/// `%`), so unsupported tokens like `%p`/`%r` degrade gracefully rather than
/// silently dropping characters. `%p`/`%r` support is tracked as a follow-up.
fn expand_hostname_tokens(template: &str, original_host: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('h') => {
                    out.push_str(original_host);
                    chars.next();
                }
                Some('%') => {
                    out.push('%');
                    chars.next();
                }
                _ => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Application onto SshTunnelConfig
// ---------------------------------------------------------------------------

/// Applies resolved `~/.ssh/config` fields onto `ssh`, without overwriting
/// values the user has explicitly set. `HostName` is token-expanded against the
/// original host before replacing `ssh.host`. `user`/`port` use the sentinel
/// defaults above to detect "not actually set by the user"; `key_path` is
/// filled only when empty, flipping `auth_method` to `"key"` when the config
/// is the sole usable credential.
fn apply_resolved_fields(ssh: &SshTunnelConfig, fields: &ResolvedHostFields) -> SshTunnelConfig {
    // No matching block set any field → nothing to do.
    if fields.host_name.is_none()
        && fields.port.is_none()
        && fields.user.is_none()
        && fields.identity_file.is_none()
    {
        return ssh.clone();
    }

    let mut resolved = ssh.clone();

    if let Some(host_name) = &fields.host_name {
        resolved.host = expand_hostname_tokens(host_name, &ssh.host);
    }
    if resolved.user == DEFAULT_USER_SENTINEL {
        if let Some(user) = &fields.user {
            resolved.user = user.clone();
        }
    }
    if resolved.port == DEFAULT_PORT_SENTINEL {
        if let Some(port) = fields.port {
            resolved.port = port;
        }
    }
    if resolved.key_path.is_empty() {
        if let Some(identity_file) = &fields.identity_file {
            resolved.key_path = identity_file.clone();
            // If the SSH config supplied the only usable credential, make the
            // backend use it even when an older/default UI payload still says
            // "password" with an empty password.
            if resolved.auth_method.is_empty()
                || (resolved.auth_method == "password" && resolved.password.is_empty())
            {
                resolved.auth_method = "key".to_string();
            }
        }
    }

    resolved
}

/// Expands a parsed config into selectable alias entries: only concrete
/// (non-negated, no `*`/`?`) patterns are emitted, each carrying the fields
/// from its own block (no cross-block first-wins baking, so wildcard-block
/// values stay out of the dropdown and are resolved dynamically at connect
/// time instead).
fn list_entries_from_blocks(blocks: &[HostBlock]) -> Vec<SshConfigHostEntry> {
    let mut entries = Vec::new();
    for block in blocks {
        for pattern in &block.patterns {
            if pattern.negated || pattern.text.contains('*') || pattern.text.contains('?') {
                continue;
            }
            entries.push(SshConfigHostEntry {
                alias: pattern.text.clone(),
                host_name: block.host_name.clone(),
                port: block.port,
                user: block.user.clone(),
                identity_file: block.identity_file.clone(),
            });
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Lexical helpers
// ---------------------------------------------------------------------------

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}

/// Splits a config line into `(keyword, value)`. OpenSSH allows the keyword
/// and value to be separated by whitespace or a single `=`.
fn split_directive(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let split_index = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let keyword = &line[..split_index];
    let value = line[split_index..]
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim();
    if keyword.is_empty() || value.is_empty() {
        return None;
    }
    Some((keyword, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(host: &str) -> SshTunnelConfig {
        SshTunnelConfig {
            profile_id: String::new(),
            id: "1".to_string(),
            name: String::new(),
            enabled: true,
            host: host.to_string(),
            port: DEFAULT_PORT_SENTINEL,
            user: DEFAULT_USER_SENTINEL.to_string(),
            password: String::new(),
            key_path: String::new(),
            key_passphrase: String::new(),
            connect_timeout_secs: 5,
            expose_lan: false,
            use_ssh_agent: false,
            ssh_agent_sock_path: String::new(),
            auth_method: "password".to_string(),
        }
    }

    /// Builds `ResolvedHostFields` from the given values, mirroring the old
    /// `entry()` helper so the resolve-path tests keep exercising the same
    /// application logic.
    fn fields(host_name: &str, port: u16, user: &str, identity_file: &str) -> ResolvedHostFields {
        ResolvedHostFields {
            host_name: Some(host_name.to_string()),
            port: Some(port),
            user: Some(user.to_string()),
            identity_file: Some(identity_file.to_string()),
        }
    }

    // --- parse_host_blocks -------------------------------------------------

    #[test]
    fn parses_basic_host_block() {
        let blocks = parse_host_blocks(
            "Host myserver\n  HostName 10.0.0.5\n  Port 2222\n  User deploy\n  IdentityFile ~/.ssh/id_ed25519\n",
        );
        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(
            block.patterns,
            vec![HostPattern {
                negated: false,
                text: "myserver".to_string()
            }]
        );
        assert_eq!(block.host_name, Some("10.0.0.5".to_string()));
        assert_eq!(block.port, Some(2222));
        assert_eq!(block.user, Some("deploy".to_string()));
        assert_eq!(block.identity_file, Some("~/.ssh/id_ed25519".to_string()));
    }

    #[test]
    fn one_line_can_declare_multiple_aliases() {
        let blocks = parse_host_blocks("Host prod prod-alias\n  HostName 10.0.0.9\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].patterns,
            vec![
                HostPattern {
                    negated: false,
                    text: "prod".to_string()
                },
                HostPattern {
                    negated: false,
                    text: "prod-alias".to_string()
                },
            ]
        );
        assert_eq!(blocks[0].host_name, Some("10.0.0.9".to_string()));
    }

    #[test]
    fn keeps_wildcard_host_patterns_as_blocks() {
        // Wildcard blocks are retained for resolution (the defect this change
        // fixes); list_entries_from_blocks is what strips them from the
        // dropdown, tested separately.
        let blocks = parse_host_blocks("Host *.example.com\n  User git\nHost real\n  User deploy\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].patterns[0].text, "*.example.com");
        assert_eq!(blocks[0].user, Some("git".to_string()));
        assert_eq!(blocks[1].patterns[0].text, "real");
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let blocks = parse_host_blocks("# a comment\n\nHost myserver # inline comment\n  User deploy\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].patterns[0].text, "myserver");
        assert_eq!(blocks[0].user, Some("deploy".to_string()));
    }

    #[test]
    fn first_value_wins_within_a_block() {
        // OpenSSH first-match semantics: the first HostName line in a block
        // wins; a later duplicate in the same block is ignored.
        let blocks = parse_host_blocks("Host srv\n  HostName first\n  HostName second\n");
        assert_eq!(blocks[0].host_name, Some("first".to_string()));
    }

    #[test]
    fn parses_negated_pattern() {
        let blocks = parse_host_blocks("Host *.corp.com !jump.corp.com\n  User ops\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].patterns.len(), 2);
        assert!(!blocks[0].patterns[0].negated);
        assert_eq!(blocks[0].patterns[0].text, "*.corp.com");
        assert!(blocks[0].patterns[1].negated);
        assert_eq!(blocks[0].patterns[1].text, "jump.corp.com");
    }

    // --- wildcard_match ----------------------------------------------------

    #[test]
    fn star_matches_any_sequence_including_empty() {
        assert!(wildcard_match("*.example.com", "web1.example.com"));
        assert!(wildcard_match("*.example.com", ".example.com"));
        assert!(!wildcard_match("*.example.com", "web1.other.com"));
    }

    #[test]
    fn question_mark_matches_exactly_one_char() {
        assert!(wildcard_match("web?.corp.com", "web1.corp.com"));
        assert!(!wildcard_match("web?.corp.com", "web12.corp.com"));
        assert!(!wildcard_match("web?.corp.com", "web.corp.com"));
    }

    #[test]
    fn literal_pattern_matches_only_itself() {
        assert!(wildcard_match("myserver", "myserver"));
        assert!(!wildcard_match("myserver", "otherserver"));
    }

    #[test]
    fn leading_and_trailing_stars() {
        assert!(wildcard_match("*prod*", "my-prod-host"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("*", ""));
    }

    // --- block_matches -----------------------------------------------------

    #[test]
    fn block_matches_on_positive_pattern() {
        let block = HostBlock {
            patterns: vec![HostPattern {
                negated: false,
                text: "*.example.com".to_string(),
            }],
            host_name: None,
            port: None,
            user: None,
            identity_file: None,
        };
        assert!(block_matches(&block, "web1.example.com"));
        assert!(!block_matches(&block, "web1.other.com"));
    }

    #[test]
    fn negated_pattern_excludes_match() {
        let block = HostBlock {
            patterns: vec![
                HostPattern {
                    negated: false,
                    text: "*.corp.com".to_string(),
                },
                HostPattern {
                    negated: true,
                    text: "jump.corp.com".to_string(),
                },
            ],
            host_name: None,
            port: None,
            user: None,
            identity_file: None,
        };
        assert!(block_matches(&block, "web1.corp.com"));
        assert!(!block_matches(&block, "jump.corp.com"));
    }

    #[test]
    fn all_negated_block_never_matches() {
        let block = HostBlock {
            patterns: vec![HostPattern {
                negated: true,
                text: "*.corp.com".to_string(),
            }],
            host_name: None,
            port: None,
            user: None,
            identity_file: None,
        };
        assert!(!block_matches(&block, "web1.corp.com"));
    }

    // --- resolve_host_fields (first-wins per field) -------------------------

    #[test]
    fn specific_block_wins_over_global_fallback() {
        let blocks = parse_host_blocks(
            "Host web1.example.com\n  User ops\n  Port 2222\nHost *\n  User default\n  Port 2345\n",
        );
        let fields = resolve_host_fields(&blocks, "web1.example.com");
        assert_eq!(fields.user, Some("ops".to_string()));
        assert_eq!(fields.port, Some(2222));
    }

    #[test]
    fn global_fallback_fills_fields_specific_block_left_unset() {
        let blocks = parse_host_blocks(
            "Host web1.example.com\n  User ops\nHost *\n  User default\n  Port 2345\n",
        );
        let fields = resolve_host_fields(&blocks, "web1.example.com");
        assert_eq!(fields.user, Some("ops".to_string()));
        // Specific block didn't set Port; global fallback fills it.
        assert_eq!(fields.port, Some(2345));
    }

    #[test]
    fn wildcard_block_backfills_user_and_identity_file() {
        let blocks =
            parse_host_blocks("Host *.prod.example.com\n  User deploy\n  IdentityFile ~/.ssh/prod_key\n");
        let fields = resolve_host_fields(&blocks, "web1.prod.example.com");
        assert_eq!(fields.user, Some("deploy".to_string()));
        assert_eq!(
            fields.identity_file,
            Some("~/.ssh/prod_key".to_string())
        );
        assert!(fields.host_name.is_none());
        assert!(fields.port.is_none());
    }

    #[test]
    fn no_matching_block_yields_all_none() {
        let blocks = parse_host_blocks("Host *.example.com\n  User git\n");
        let fields = resolve_host_fields(&blocks, "totally-unrelated.host");
        assert_eq!(fields, ResolvedHostFields::default());
    }

    #[test]
    fn duplicate_alias_blocks_take_first_occurrence() {
        // Fixes the prior last-wins quirk: the first Host block for an alias
        // provides its fields; a later block with the same alias is ignored.
        let blocks = parse_host_blocks(
            "Host srv\n  User first\n  Port 1111\nHost srv\n  User second\n  Port 2222\n",
        );
        let fields = resolve_host_fields(&blocks, "srv");
        assert_eq!(fields.user, Some("first".to_string()));
        assert_eq!(fields.port, Some(1111));
    }

    // --- expand_hostname_tokens --------------------------------------------

    #[test]
    fn hostname_expands_percent_h_token() {
        assert_eq!(
            expand_hostname_tokens("%h.internal", "web1.example.com"),
            "web1.example.com.internal"
        );
    }

    #[test]
    fn hostname_expands_double_percent_as_literal() {
        assert_eq!(
            expand_hostname_tokens("10.0.0.5%%suffix", "web1"),
            "10.0.0.5%suffix"
        );
    }

    #[test]
    fn hostname_combines_tokens() {
        assert_eq!(
            expand_hostname_tokens("%h-%h.%%", "web1"),
            "web1-web1.%"
        );
    }

    #[test]
    fn unsupported_token_passes_through_literal() {
        // %p / %r are not supported; the `%` is preserved so the value degrades
        // visibly rather than silently losing characters.
        assert_eq!(expand_hostname_tokens("host%p", "web1"), "host%p");
    }

    // --- apply_resolved_fields (preserved application logic) ----------------

    #[test]
    fn resolve_fills_unset_fields_from_matching_alias() {
        let ssh = config("myserver");
        let resolved = apply_resolved_fields(&ssh, &fields("10.0.0.5", 2222, "deploy", "~/.ssh/id_ed25519"));
        assert_eq!(resolved.host, "10.0.0.5");
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.user, "deploy");
        assert_eq!(resolved.key_path, "~/.ssh/id_ed25519");
        assert_eq!(resolved.auth_method, "key");
    }

    #[test]
    fn resolve_keeps_password_auth_when_password_is_present() {
        let mut ssh = config("myserver");
        ssh.password = "secret".to_string();
        let resolved = apply_resolved_fields(&ssh, &fields("10.0.0.5", 2222, "deploy", "~/.ssh/id_ed25519"));
        assert_eq!(resolved.key_path, "~/.ssh/id_ed25519");
        assert_eq!(resolved.auth_method, "password");
    }

    #[test]
    fn resolve_preserves_key_plus_password_method() {
        let mut ssh = config("myserver");
        ssh.auth_method = "key+password".to_string();
        ssh.password = "secret".to_string();
        let resolved = apply_resolved_fields(&ssh, &fields("10.0.0.5", 2222, "deploy", "~/.ssh/id_ed25519"));
        assert_eq!(resolved.key_path, "~/.ssh/id_ed25519");
        assert_eq!(resolved.auth_method, "key+password");
    }

    #[test]
    fn resolve_does_not_override_explicit_values() {
        let mut ssh = config("myserver");
        ssh.user = "alice".to_string();
        ssh.port = 9999;
        ssh.key_path = "/explicit/key".to_string();
        let resolved = apply_resolved_fields(&ssh, &fields("10.0.0.5", 2222, "deploy", "~/.ssh/id_ed25519"));
        assert_eq!(resolved.host, "10.0.0.5");
        assert_eq!(resolved.user, "alice");
        assert_eq!(resolved.port, 9999);
        assert_eq!(resolved.key_path, "/explicit/key");
    }

    #[test]
    fn resolve_is_noop_when_host_does_not_match_any_alias() {
        // `resolve_ssh_tunnel_config` looks up the real `~/.ssh/config`; an
        // alias this unlikely to exist on a test machine exercises the "no
        // match found" branch without needing to mock the filesystem.
        let ssh = config("dbx-test-alias-that-should-never-exist-anywhere");
        let resolved = resolve_ssh_tunnel_config(&ssh);
        assert_eq!(resolved, ssh);
    }

    // --- list_entries_from_blocks (dropdown shape) -------------------------

    #[test]
    fn list_excludes_wildcard_and_negated_patterns() {
        let blocks = parse_host_blocks(
            "Host *.example.com\n  User git\nHost real !evil\n  User deploy\nHost * \n  User global\n",
        );
        let entries = list_entries_from_blocks(&blocks);
        // Only the concrete non-negated pattern `real` survives; `*.example.com`,
        // the negated `evil`, and `*` are all dropped.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "real");
        assert_eq!(entries[0].user, Some("deploy".to_string()));
    }

    #[test]
    fn list_entries_carry_only_their_own_block_fields() {
        // A concrete alias in a wildcard-heavy config carries its block's
        // fields only — values from a later `Host *` fallback are not baked in,
        // so the dropdown shows per-block values and resolve fills the rest.
        let blocks = parse_host_blocks(
            "Host srv\n  HostName 10.0.0.5\nHost *\n  User global\n  Port 2345\n",
        );
        let entries = list_entries_from_blocks(&blocks);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "srv");
        assert_eq!(entries[0].host_name, Some("10.0.0.5".to_string()));
        // global-block fields are not baked into the `srv` entry.
        assert_eq!(entries[0].user, None);
        assert_eq!(entries[0].port, None);
    }

    #[test]
    fn list_expands_multiple_aliases_in_one_block() {
        let blocks = parse_host_blocks("Host prod prod-alias\n  HostName 10.0.0.9\n");
        let entries = list_entries_from_blocks(&blocks);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.host_name == Some("10.0.0.9".to_string())));
        assert_eq!(entries[0].alias, "prod");
        assert_eq!(entries[1].alias, "prod-alias");
    }

    // --- end-to-end resolve via parse_host_blocks + resolve_host_fields -----

    #[test]
    fn resolve_via_wildcard_block_applies_user_port_identity_file() {
        // The core defect this change fixes: a wildcard Host block supplies
        // user/port/identity_file for a concrete host that matches it.
        let blocks = parse_host_blocks(
            "Host *.prod.example.com\n  User ops\n  Port 2222\n  IdentityFile ~/.ssh/prod_key\n",
        );
        let fields = resolve_host_fields(&blocks, "web1.prod.example.com");
        let ssh = config("web1.prod.example.com");
        let resolved = apply_resolved_fields(&ssh, &fields);
        assert_eq!(resolved.user, "ops");
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.key_path, "~/.ssh/prod_key");
        assert_eq!(resolved.auth_method, "key");
    }

    #[test]
    fn resolve_hostname_percent_h_in_wildcard_block() {
        // HostName `%h.internal` in a wildcard block expands against the
        // original host the user typed.
        let blocks = parse_host_blocks(
            "Host *.example.com\n  HostName %h.internal\n",
        );
        let fields = resolve_host_fields(&blocks, "web1.example.com");
        let ssh = config("web1.example.com");
        let resolved = apply_resolved_fields(&ssh, &fields);
        assert_eq!(resolved.host, "web1.example.com.internal");
    }

    #[test]
    fn resolve_global_fallback_only_fills_unset_fields() {
        let blocks = parse_host_blocks(
            "Host web1\n  User ops\nHost *\n  User default\n  Port 2345\n",
        );
        let fields = resolve_host_fields(&blocks, "web1");
        let mut ssh = config("web1");
        ssh.user = "explicit".to_string();
        let resolved = apply_resolved_fields(&ssh, &fields);
        // Explicit user preserved; global fallback fills port only.
        assert_eq!(resolved.user, "explicit");
        assert_eq!(resolved.port, 2345);
    }
}
