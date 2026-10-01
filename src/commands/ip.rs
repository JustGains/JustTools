//! Reporting the public IP address this machine presents to the internet.
//!
//! Both families are looked up at once so a bare run answers the question
//! without a switch. Each family is pinned by hostname rather than by socket
//! option: a name that publishes records for one family can only be reached
//! over that family, so the address the service echoes back is the one it
//! actually saw. A machine with no public route for a family is reported
//! straight away instead of waiting out a connection that cannot complete.

use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::common;
use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justip";

const HELP: &str = r#"justip — Show the public IP address this machine presents.

Usage:
  justip [options]

Both IPv4 and IPv6 are reported by default. The address is echoed back by a
public lookup service, so it is what the internet sees rather than a local
interface address. Only the request itself is sent.

Options:
  -4, --ipv4             Report only the public IPv4 address
  -6, --ipv6             Report only the public IPv6 address
      --plain            Print bare addresses only, one per line
      --json             Print machine-readable JSON
  -t, --timeout SECONDS  Seconds to wait for each lookup (1-60, default: 5)
  -h, --help             Show this help

Run bare to open the interactive launcher; explicit arguments bypass the UI.
A family this machine cannot reach is reported as unavailable, and the run
fails only when no requested family answers."#;

/// Public echo services, tried in order until one answers usefully.
///
/// Each hostname publishes records for a single family, and that is what pins
/// the request to that family. Later entries are independent operators so one
/// outage does not take the tool down with it.
const IPV4_SERVICES: &[&str] = &[
    "https://api.ipify.org",
    "https://ipv4.icanhazip.com",
    "https://v4.ident.me",
];
const IPV6_SERVICES: &[&str] = &[
    "https://api6.ipify.org",
    "https://ipv6.icanhazip.com",
    "https://v6.ident.me",
];

/// An echoed address is one short line, so anything longer is not an answer.
const MAX_RESPONSE_BYTES: u64 = 128;
const DEFAULT_TIMEOUT: u32 = 5;
const MAX_REASON_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn label(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::V4 => "ipv4",
            Self::V6 => "ipv6",
        }
    }

    fn matches(self, address: IpAddr) -> bool {
        matches!(
            (self, address),
            (Self::V4, IpAddr::V4(_)) | (Self::V6, IpAddr::V6(_))
        )
    }

    /// The services to ask, in the order they are tried.
    ///
    /// An environment override replaces the whole list, semicolons between
    /// entries, so a test can point at local servers instead of reaching the
    /// network and still exercise the fallback chain.
    fn services(self) -> Services {
        let variable = match self {
            Self::V4 => "JUSTIP_IPV4_URL",
            Self::V6 => "JUSTIP_IPV6_URL",
        };
        if let Ok(overridden) = std::env::var(variable) {
            let urls: Vec<String> = overridden
                .split(';')
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_owned)
                .collect();
            if !urls.is_empty() {
                return Services {
                    urls,
                    overridden: true,
                };
            }
        }
        Services {
            urls: match self {
                Self::V4 => IPV4_SERVICES,
                Self::V6 => IPV6_SERVICES,
            }
            .iter()
            .map(|url| (*url).to_owned())
            .collect(),
            overridden: false,
        }
    }

    /// Where to bind and which address to route towards when asking the
    /// routing table for a source address.
    ///
    /// Both destinations are documentation prefixes. Connecting a UDP socket
    /// only selects a route and a source address; no packet is ever sent, so
    /// the address chosen here is never contacted.
    fn probe(self) -> (&'static str, &'static str) {
        match self {
            Self::V4 => ("0.0.0.0:0", "192.0.2.1:53"),
            Self::V6 => ("[::]:0", "[2001:db8::1]:53"),
        }
    }
}

struct Services {
    urls: Vec<String>,
    /// An override points somewhere other than the public internet, so the
    /// public-route check does not describe it.
    overridden: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Labeled,
    Plain,
    Json,
}

#[derive(Debug)]
struct Options {
    ipv4: bool,
    ipv6: bool,
    format: Format,
    timeout: u32,
    help: bool,
}

impl Options {
    /// Neither flag and both flags mean the same thing: report everything.
    fn families(&self) -> Vec<Family> {
        match (self.ipv4, self.ipv6) {
            (true, false) => vec![Family::V4],
            (false, true) => vec![Family::V6],
            _ => vec![Family::V4, Family::V6],
        }
    }
}

fn parse(args: Vec<OsString>) -> ToolResult<Options> {
    let mut options = Options {
        ipv4: false,
        ipv6: false,
        format: Format::Labeled,
        timeout: DEFAULT_TIMEOUT,
        help: false,
    };
    let mut index = 0;
    while index < args.len() {
        let original = common::os_to_string(TOOL, &args[index], "option")?;
        let (option, inline) = original
            .split_once('=')
            .filter(|_| original.starts_with("--"))
            .map_or((original.as_str(), None), |(key, value)| {
                (key, Some(value.to_owned()))
            });
        match option {
            "-h" | "--help" => options.help = true,
            "-4" | "--ipv4" => options.ipv4 = true,
            "-6" | "--ipv6" => options.ipv6 = true,
            "--plain" => options.format = Format::Plain,
            "--json" => options.format = Format::Json,
            "-t" | "--timeout" => {
                let value = match inline {
                    Some(value) => value,
                    None => common::option_value(TOOL, &args, &mut index, option)?,
                };
                options.timeout = common::integer(TOOL, &value, "timeout", 1, 60)?;
            }
            _ if option.starts_with('-') => {
                return Err(ToolError::usage(TOOL, format!("unknown option: {option}")));
            }
            _ => {
                return Err(ToolError::usage(
                    TOOL,
                    format!("justip takes no arguments: {option}"),
                ));
            }
        }
        index += 1;
    }
    Ok(options)
}

/// Whether the routing table offers a source address that can reach a public
/// service for `family`.
///
/// Waiting out a connection to an unreachable family is the slowest part of a
/// bare run, and an IPv6 stack limited to link-local or unique-local addresses
/// never had a path to begin with. Reading the source address a UDP connect
/// selects costs nothing and sends nothing.
fn reachable(family: Family) -> bool {
    let (bind, probe) = family.probe();
    let (Ok(socket), Ok(probe)) = (UdpSocket::bind(bind), probe.parse::<SocketAddr>()) else {
        return false;
    };
    if socket.connect(probe).is_err() {
        return false;
    }
    match socket.local_addr() {
        // Behind NAT the IPv4 source is private, which says nothing about
        // reachability; only loopback and an unassigned socket rule it out.
        Ok(SocketAddr::V4(local)) => !local.ip().is_loopback() && !local.ip().is_unspecified(),
        // Global unicast is 2000::/3. A unique-local or link-local source
        // cannot reach a public service however the route is configured.
        Ok(SocketAddr::V6(local)) => local.ip().segments()[0] & 0xE000 == 0x2000,
        Err(_) => false,
    }
}

fn shorten(reason: &str) -> String {
    let mut text = reason.replace(['\r', '\n', '\0'], " ");
    if text.chars().count() > MAX_REASON_CHARS {
        let boundary = text
            .char_indices()
            .nth(MAX_REASON_CHARS)
            .map_or(text.len(), |(index, _)| index);
        text.truncate(boundary);
        text.push_str("...");
    }
    text
}

fn fetch(url: &str, timeout: Duration) -> Result<IpAddr, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|error| error.to_string())?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|error| error.to_string())?;
    body.trim()
        .parse::<IpAddr>()
        .map_err(|_| "answer was not an IP address".to_owned())
}

fn lookup(family: Family, timeout: Duration) -> Result<IpAddr, String> {
    let services = family.services();
    if !services.overridden && !reachable(family) {
        return Err(format!(
            "this machine has no public {} route",
            family.label()
        ));
    }
    let mut failure = format!("no {} service was reachable", family.label());
    for url in services.urls {
        match fetch(&url, timeout) {
            Ok(address) if family.matches(address) => return Ok(address),
            // A service answering with the other family cannot be trusted for
            // this one, so move on rather than report the wrong address.
            Ok(_) => failure = format!("{url} answered with the wrong address family"),
            Err(error) => failure = format!("{url}: {error}"),
        }
    }
    Err(shorten(&failure))
}

struct Report {
    family: Family,
    result: Result<IpAddr, String>,
}

fn emit(reports: &[Report], format: Format) {
    match format {
        Format::Labeled => {
            for report in reports {
                match &report.result {
                    Ok(address) => println!("{}: {address}", report.family.label()),
                    Err(reason) => {
                        println!("{}: unavailable ({reason})", report.family.label());
                    }
                }
            }
        }
        // Plain and JSON are consumed by other programs, so stdout carries only
        // the answer and every explanation goes to stderr.
        Format::Plain => {
            for report in reports {
                match &report.result {
                    Ok(address) => println!("{address}"),
                    Err(reason) => {
                        eprintln!("{TOOL}: {} unavailable: {reason}", report.family.label());
                    }
                }
            }
        }
        Format::Json => {
            let mut object = Map::new();
            for report in reports {
                let value = match &report.result {
                    Ok(address) => Value::String(address.to_string()),
                    Err(reason) => {
                        eprintln!("{TOOL}: {} unavailable: {reason}", report.family.label());
                        Value::Null
                    }
                };
                object.insert(report.family.key().to_owned(), value);
            }
            println!("{}", Value::Object(object));
        }
    }
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    let timeout = Duration::from_secs(u64::from(options.timeout));
    // Both families are looked up together so reporting both costs one
    // timeout rather than two.
    let reports: Vec<Report> = std::thread::scope(|scope| {
        let handles: Vec<_> = options
            .families()
            .into_iter()
            .map(|family| (family, scope.spawn(move || lookup(family, timeout))))
            .collect();
        handles
            .into_iter()
            .map(|(family, handle)| Report {
                family,
                result: handle
                    .join()
                    .unwrap_or_else(|_| Err("the lookup did not finish".to_owned())),
            })
            .collect()
    });
    let failures: Vec<String> = reports
        .iter()
        .filter_map(|report| {
            report
                .result
                .as_ref()
                .err()
                .map(|reason| format!("{}: {reason}", report.family.label()))
        })
        .collect();
    if failures.len() == reports.len() {
        return Err(ToolError::new(
            TOOL,
            format!("no public address available ({})", failures.join("; ")),
        ));
    }
    emit(&reports, options.format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> ToolResult<Options> {
        parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn both_families_are_reported_without_a_switch() {
        let options = parse_args(&[]).unwrap();
        assert_eq!(options.families(), vec![Family::V4, Family::V6]);
        assert_eq!(options.format, Format::Labeled);
        assert_eq!(options.timeout, DEFAULT_TIMEOUT);
        // Asking for both explicitly is the same request as asking for neither.
        assert_eq!(
            parse_args(&["-4", "-6"]).unwrap().families(),
            vec![Family::V4, Family::V6]
        );
        assert_eq!(parse_args(&["-4"]).unwrap().families(), vec![Family::V4]);
        assert_eq!(
            parse_args(&["--ipv6"]).unwrap().families(),
            vec![Family::V6]
        );
    }

    #[test]
    fn output_and_timeout_options_parse_in_both_forms() {
        assert_eq!(parse_args(&["--plain"]).unwrap().format, Format::Plain);
        assert_eq!(parse_args(&["--json"]).unwrap().format, Format::Json);
        assert_eq!(parse_args(&["--timeout=12"]).unwrap().timeout, 12);
        assert_eq!(parse_args(&["-t", "30"]).unwrap().timeout, 30);
        for rejected in [
            vec!["--timeout", "0"],
            vec!["--timeout", "61"],
            vec!["--timeout"],
            vec!["--timeout="],
            vec!["--nope"],
            vec!["8.8.8.8"],
        ] {
            let error = parse_args(&rejected).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{rejected:?} should be a usage error");
        }
    }

    #[test]
    fn an_answer_is_only_accepted_for_the_family_it_was_asked_for() {
        assert!(Family::V4.matches("203.0.113.7".parse().unwrap()));
        assert!(!Family::V4.matches("2001:db8::7".parse().unwrap()));
        assert!(Family::V6.matches("2001:db8::7".parse().unwrap()));
        assert!(!Family::V6.matches("203.0.113.7".parse().unwrap()));
    }

    #[test]
    fn a_long_failure_is_trimmed_to_one_line() {
        assert_eq!(shorten("dns failed\nfor host"), "dns failed for host");
        let long = shorten(&"x".repeat(MAX_REASON_CHARS + 50));
        assert_eq!(long.chars().count(), MAX_REASON_CHARS + 3);
        assert!(long.ends_with("..."));
        // A message that already fits is returned unchanged.
        assert_eq!(shorten("short"), "short");
    }

    #[test]
    fn an_ipv4_route_exists_wherever_tests_run() {
        // Every test host can reach its own loopback-free IPv4 stack; IPv6 is
        // deliberately not asserted because runners differ.
        assert!(reachable(Family::V4));
    }
}
