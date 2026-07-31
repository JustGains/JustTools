use crate::common::{confirm, parse_cli, stdin_is_terminal, stdout_is_terminal};
use anyhow::{Result, anyhow, bail};
use clap::Parser;
use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use sysinfo::{Pid, System};

#[derive(Debug, Parser)]
#[command(
    name = "justport",
    about = "Find what is using a local port.",
    after_help = "Examples:\n  justport 3000 5000\n  justport --all\n  justport --kill 4321"
)]
struct Cli {
    /// Stop owning user processes (system PIDs are protected).
    #[arg(short = 'k', long)]
    kill: bool,

    /// Include UDP endpoints.
    #[arg(short = 'a', long)]
    all: bool,

    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,

    /// Skip kill confirmation.
    #[arg(short = 'y', long)]
    yes: bool,

    /// Exact local ports to inspect.
    #[arg(value_name = "PORT", value_parser = parse_port)]
    ports: Vec<u16>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Endpoint {
    protocol: String,
    port: u16,
    address: String,
    state: String,
    #[serde(rename = "PID")]
    pid: u32,
    process: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct PortResult<'a> {
    port: u16,
    available: bool,
    endpoints: Vec<&'a Endpoint>,
}

#[derive(Debug)]
struct OwnerIdentity {
    pid: u32,
    name: String,
    start_time: u64,
    listeners: BTreeSet<(String, u16)>,
}

pub fn run() -> Result<()> {
    let Some(options) = parse_cli::<Cli>()? else {
        return Ok(());
    };
    run_with(options)
}

fn run_with(mut options: Cli) -> Result<()> {
    options.ports.sort_unstable();
    options.ports.dedup();
    if options.kill && options.ports.is_empty() {
        bail!("--kill requires at least one port");
    }
    if options.kill && options.json {
        bail!("--kill cannot be combined with --json");
    }

    let system = System::new_all();
    let mut endpoints = socket_endpoints(&options, &system)?;
    endpoints.sort_by(|left, right| {
        (left.port, &left.protocol, &left.address, left.pid).cmp(&(
            right.port,
            &right.protocol,
            &right.address,
            right.pid,
        ))
    });
    endpoints.dedup_by(|left, right| {
        left.port == right.port
            && left.protocol == right.protocol
            && left.address == right.address
            && left.state == right.state
            && left.pid == right.pid
    });

    if options.kill {
        return kill_owners(&endpoints, &system, options.yes);
    }
    if options.json {
        if options.ports.is_empty() {
            println!("{}", serde_json::to_string_pretty(&endpoints)?);
        } else {
            let results: Vec<_> = options
                .ports
                .iter()
                .map(|port| {
                    let matches: Vec<_> =
                        endpoints.iter().filter(|row| row.port == *port).collect();
                    PortResult {
                        port: *port,
                        available: matches.is_empty(),
                        endpoints: matches,
                    }
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        return Ok(());
    }

    if !options.ports.is_empty() {
        for port in &options.ports {
            let matches: Vec<_> = endpoints.iter().filter(|row| row.port == *port).collect();
            if matches.is_empty() {
                println!("{port}: free");
                continue;
            }
            for row in matches {
                let state = if row.state == "-" {
                    String::new()
                } else {
                    format!(" {}", row.state)
                };
                println!(
                    "{}: {} {}{}  PID {} ({})",
                    port, row.protocol, row.address, state, row.pid, row.process
                );
            }
        }
        return Ok(());
    }

    if endpoints.is_empty() {
        println!("justport: no listening ports");
        return Ok(());
    }
    println!("PROTO PORT  ADDRESS                    PID PROCESS");
    for row in endpoints {
        println!(
            "{:<5} {:>5}  {:<25} {:>6} {}",
            row.protocol, row.port, row.address, row.pid, row.process
        );
    }
    Ok(())
}

fn socket_endpoints(options: &Cli, system: &System) -> Result<Vec<Endpoint>> {
    let address_flags = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
    let mut protocol_flags = ProtocolFlags::TCP;
    if options.all {
        protocol_flags |= ProtocolFlags::UDP;
    }
    let selected: HashSet<_> = options.ports.iter().copied().collect();
    let exact_ports = !selected.is_empty();
    let sockets = get_sockets_info(address_flags, protocol_flags)
        .map_err(|error| anyhow!("could not inspect local sockets: {error}"))?;
    let mut endpoints = Vec::new();
    for socket in sockets {
        let (protocol, port, address, state, include) = match socket.protocol_socket_info {
            ProtocolSocketInfo::Tcp(tcp) => (
                "TCP",
                tcp.local_port,
                tcp.local_addr.to_string(),
                format!("{:?}", tcp.state),
                if exact_ports {
                    selected.contains(&tcp.local_port)
                } else {
                    tcp.state == TcpState::Listen
                },
            ),
            ProtocolSocketInfo::Udp(udp) => (
                "UDP",
                udp.local_port,
                udp.local_addr.to_string(),
                "-".to_owned(),
                !exact_ports || selected.contains(&udp.local_port),
            ),
        };
        if !include {
            continue;
        }
        if socket.associated_pids.is_empty() {
            endpoints.push(Endpoint {
                protocol: protocol.to_owned(),
                port,
                address,
                state,
                pid: 0,
                process: "<unknown>".to_owned(),
            });
            continue;
        }
        for pid in socket.associated_pids {
            endpoints.push(Endpoint {
                protocol: protocol.to_owned(),
                port,
                address: address.clone(),
                state: state.clone(),
                pid,
                process: process_name(system, pid),
            });
        }
    }
    Ok(endpoints)
}

fn process_name(system: &System, pid: u32) -> String {
    system
        .process(Pid::from_u32(pid))
        .map(|process| process.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| "<ended>".to_owned())
}

fn kill_owners(endpoints: &[Endpoint], system: &System, yes: bool) -> Result<()> {
    let listeners: Vec<_> = endpoints
        .iter()
        .filter(|row| row.protocol == "UDP" || row.state == "Listen")
        .collect();
    let current = std::process::id();
    let current_process = system
        .process(Pid::from_u32(current))
        .ok_or_else(|| anyhow!("could not identify the current process safely"))?;
    let current_user = current_process.user_id();
    let mut protected = Vec::new();
    let mut stale = Vec::new();
    let mut owners: BTreeMap<u32, OwnerIdentity> = BTreeMap::new();
    for row in listeners.iter().filter(|row| row.pid > 0) {
        let Some(process) = system.process(Pid::from_u32(row.pid)) else {
            stale.push(format!("{}/{} PID {}", row.protocol, row.port, row.pid));
            continue;
        };
        let owned_by_current_user = current_user.is_some() && process.user_id() == current_user;
        if row.pid <= 4 || row.pid == current || !owned_by_current_user {
            protected.push(*row);
            continue;
        }
        let owner = owners.entry(row.pid).or_insert_with(|| OwnerIdentity {
            pid: row.pid,
            name: process.name().to_string_lossy().into_owned(),
            start_time: process.start_time(),
            listeners: BTreeSet::new(),
        });
        owner.listeners.insert((row.protocol.clone(), row.port));
    }
    if !stale.is_empty() {
        bail!(
            "socket ownership changed while inspecting {}; retry the command",
            stale.join(", ")
        );
    }
    if !protected.is_empty() {
        let details = protected
            .iter()
            .map(|row| format!("{}/{} PID {}", row.protocol, row.port, row.pid))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("refusing to stop a process not owned by the current user: {details}");
    }
    if owners.is_empty() {
        bail!("no process is using requested port(s)");
    }
    if !yes {
        if !stdin_is_terminal() || !stdout_is_terminal() {
            bail!("confirmation requires a terminal; re-run with --yes");
        }
        let description = owners
            .values()
            .map(|owner| format!("{} (PID {})", owner.name, owner.pid))
            .collect::<Vec<_>>()
            .join(", ");
        if !confirm(&format!("justport: stop {description}"))? {
            bail!("cancelled");
        }
    }
    for owner in owners.into_values() {
        // Refresh both process identity and socket ownership immediately before
        // each kill. This prevents a stale PID snapshot from targeting a new,
        // unrelated process after PID reuse.
        let refreshed = System::new_all();
        let process = refreshed.process(Pid::from_u32(owner.pid)).ok_or_else(|| {
            anyhow!(
                "{} (PID {}) ended before it could be stopped",
                owner.name,
                owner.pid
            )
        })?;
        let refreshed_name = process.name().to_string_lossy();
        if !same_process_identity(
            &owner.name,
            owner.start_time,
            &refreshed_name,
            process.start_time(),
        ) || process.user_id() != current_user
        {
            bail!(
                "refusing to stop PID {} because its process identity changed; retry the command",
                owner.pid
            );
        }
        if !still_owns_listener(&owner)? {
            bail!(
                "refusing to stop {} (PID {}) because it no longer owns the requested port; retry the command",
                owner.name,
                owner.pid
            );
        }
        if !process.kill() {
            bail!(
                "could not stop {} (PID {}); check permissions",
                owner.name,
                owner.pid
            );
        }
        println!("justport: stopped {} (PID {})", owner.name, owner.pid);
    }
    Ok(())
}

fn same_process_identity(
    expected_name: &str,
    expected_start_time: u64,
    actual_name: &str,
    actual_start_time: u64,
) -> bool {
    expected_name == actual_name && expected_start_time == actual_start_time
}

fn still_owns_listener(owner: &OwnerIdentity) -> Result<bool> {
    let sockets = get_sockets_info(
        AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
        ProtocolFlags::TCP | ProtocolFlags::UDP,
    )
    .map_err(|error| anyhow!("could not revalidate local sockets: {error}"))?;
    Ok(sockets.into_iter().any(|socket| {
        if !socket.associated_pids.contains(&owner.pid) {
            return false;
        }
        let listener = match socket.protocol_socket_info {
            ProtocolSocketInfo::Tcp(tcp) if tcp.state == TcpState::Listen => {
                ("TCP".to_owned(), tcp.local_port)
            }
            ProtocolSocketInfo::Udp(udp) => ("UDP".to_owned(), udp.local_port),
            _ => return false,
        };
        owner.listeners.contains(&listener)
    }))
}

fn parse_port(input: &str) -> std::result::Result<u16, String> {
    let port = input
        .parse::<u16>()
        .map_err(|_| format!("port must be an integer from 1 to 65535: {input}"))?;
    if port == 0 {
        return Err(format!("port must be an integer from 1 to 65535: {input}"));
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ports() {
        assert_eq!(parse_port("4321").unwrap(), 4321);
        assert!(parse_port("0").is_err());
        assert!(parse_port("65536").is_err());
    }

    #[test]
    fn json_contract_uses_existing_field_names() {
        let endpoint = Endpoint {
            protocol: "TCP".into(),
            port: 3000,
            address: "127.0.0.1".into(),
            state: "Listen".into(),
            pid: 42,
            process: "test".into(),
        };
        let value = serde_json::to_value(endpoint).unwrap();
        assert_eq!(value["Protocol"], "TCP");
        assert_eq!(value["Port"], 3000);
        assert_eq!(value["PID"], 42);
    }

    #[test]
    fn process_identity_rejects_pid_reuse() {
        assert!(same_process_identity("server", 100, "server", 100));
        assert!(!same_process_identity("server", 100, "server", 101));
        assert!(!same_process_identity("server", 100, "other", 100));
    }
}
