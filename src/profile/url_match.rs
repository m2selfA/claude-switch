pub(super) const NATIVE_SEARCH_URLS: &[&str] = &[
    "https://api.deepseek.com/anthropic", // DeepSeek: has search, lacks fetch
    "https://a-ocnfniawgw.cn-shanghai.fcapp.run", // AnyRouter: has both
    "https://anyrouter.top",              // AnyRouter: has both
    "https://api.anthropic.com",          // Claude official: has both
];

pub(super) const NATIVE_FETCH_URLS: &[&str] = &[
    "https://a-ocnfniawgw.cn-shanghai.fcapp.run",
    "https://anyrouter.top",
    "https://api.anthropic.com",
];

pub(super) const ANYROUTER_URLS: &[&str] = &[
    "https://a-ocnfniawgw.cn-shanghai.fcapp.run",
    "https://anyrouter.top",
];

pub(super) fn url_matches(url: &str, known: &[&str]) -> bool {
    let Some(u) = canonical_url_for_match(url) else {
        return false;
    };
    known.iter().any(|known_url| {
        canonical_url_for_match(known_url)
            .is_some_and(|k| u == k || u.starts_with(&format!("{k}/")))
    })
}

pub(crate) fn base_url_host(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?;
    let (host, _) = split_host_port(authority);
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some(
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase(),
    )
}

pub(crate) fn is_local_runtime_base_url(base_url: &str) -> bool {
    let Some(host) = base_url_host(base_url) else {
        return false;
    };
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || is_private_172_host(&host)
}

fn is_private_172_host(host: &str) -> bool {
    let Some(remainder) = host.strip_prefix("172.") else {
        return false;
    };
    let Some((octet, _)) = remainder.split_once('.') else {
        return false;
    };
    octet
        .parse::<u8>()
        .ok()
        .is_some_and(|value| (16..=31).contains(&value))
}

fn canonical_url_for_match(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let (scheme, rest) = trimmed.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let path_and_more = &rest[authority_end..];
    let host_port = authority.rsplit('@').next()?;
    let (host, port) = split_host_port(host_port);
    if host.is_empty() {
        return None;
    }

    let host = host.to_ascii_lowercase();
    let authority = match port {
        Some("") => host,
        Some(port) if !port.chars().all(|ch| ch.is_ascii_digit()) => return None,
        Some(port) if is_default_port(&scheme, port) => host,
        Some(port) => format!("{host}:{port}"),
        None => host,
    };

    Some(format!("{scheme}://{authority}{path_and_more}"))
}

fn split_host_port(authority: &str) -> (&str, Option<&str>) {
    if let Some(close_bracket) = authority.find(']')
        && authority.starts_with('[')
    {
        let host = &authority[..=close_bracket];
        let rest = &authority[close_bracket + 1..];
        return rest
            .strip_prefix(':')
            .map_or((host, None), |port| (host, Some(port)));
    }

    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.contains(':')
    {
        return (host, Some(port));
    }

    (authority, None)
}

fn is_default_port(scheme: &str, port: &str) -> bool {
    matches!((scheme, port), ("http", "80") | ("https", "443"))
}
