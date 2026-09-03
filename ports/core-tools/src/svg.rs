use crate::common::{
    absolute_lexical, atomic_write, collect_files, confirm, display_path, file_name, format_bytes,
    parse_cli, read_stdin, stdin_is_terminal, validate_unique_outputs,
};
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use oxvg_ast::{parse::roxmltree::parse, serialize::Node as _, visitor::Info};
use oxvg_optimiser::Jobs;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "justsvg",
    about = "Optimize SVGs with a Rust-native SVGOMG-style engine.",
    after_help = "The conservative preset preserves viewBox, IDs, title, description, XML namespaces,\nand accessibility attributes. Files are replaced only when optimization makes them smaller.\nThe installed JustTools multicall alias opens a saved-defaults launcher when run bare."
)]
struct Cli {
    /// Decimal precision, 0-5 (default: 3).
    #[arg(short = 'p', long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=5))]
    precision: u8,

    /// Disable multipass optimization.
    #[arg(long)]
    single_pass: bool,

    /// Write copies to DIR and keep source files.
    #[arg(short = 'o', long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Include nested folders.
    #[arg(short = 'r', long)]
    recursive: bool,

    /// Skip a folder-scan overwrite confirmation.
    #[arg(short = 'y', long)]
    yes: bool,

    /// Show inputs and outputs without writing.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// SVG files or folders. Defaults to the current folder on a terminal.
    #[arg(value_name = "FILE-OR-FOLDER")]
    inputs: Vec<PathBuf>,
}

pub fn run() -> Result<()> {
    let Some(options) = parse_cli::<Cli>()? else {
        return Ok(());
    };
    run_with(options)
}

fn run_with(mut options: Cli) -> Result<()> {
    if options.inputs.is_empty() && !stdin_is_terminal() {
        let text = read_stdin()?;
        if text.trim().is_empty() {
            bail!("no SVG received on stdin");
        }
        print!(
            "{}",
            optimize_svg(&text, "stdin.svg", options.precision, !options.single_pass)?
        );
        return Ok(());
    }
    if options.inputs.is_empty() {
        options
            .inputs
            .push(std::env::current_dir().context("could not determine current folder")?);
    }

    let output = options
        .output
        .as_deref()
        .map(absolute_lexical)
        .transpose()?;
    let collected = collect_files(&options.inputs, "svg", options.recursive, output.as_deref())?;
    for warning in &collected.warnings {
        eprintln!("justsvg: {warning}");
    }
    if collected.files.is_empty() {
        bail!("no SVG files found");
    }
    let plans: Vec<_> = collected
        .files
        .iter()
        .map(|source| {
            let destination = if let Some(directory) = output.as_ref() {
                directory.join(file_name(source)?)
            } else {
                source.clone()
            };
            Ok((source.clone(), destination))
        })
        .collect::<Result<_>>()?;
    validate_unique_outputs(&plans)?;

    if options.dry_run {
        println!("justsvg: dry run — {} file(s)", plans.len());
        for (source, destination) in &plans {
            println!(
                "  {} -> {}",
                display_path(source),
                display_path(destination)
            );
        }
        return Ok(());
    }
    if collected.used_directory
        && !options.yes
        && plans.iter().any(|(_, destination)| destination.exists())
        && !confirm(&format!(
            "justsvg: optimize or replace {} file(s)",
            plans.len()
        ))?
    {
        bail!("cancelled");
    }

    let mut changed = 0usize;
    let mut saved = 0u64;
    for (source, destination) in &plans {
        let original = fs::read_to_string(source)
            .with_context(|| format!("could not read {}", display_path(source)))?;
        let optimized = optimize_svg(
            &original,
            &display_path(source),
            options.precision,
            !options.single_pass,
        )?;
        let before = original.len() as u64;
        let after = optimized.len() as u64;
        if output.is_none() && after >= before {
            println!(
                "  kept    {} ({})",
                display_path(source),
                if after == before {
                    "same size"
                } else {
                    "optimized SVG is larger"
                }
            );
            continue;
        }
        atomic_write(destination, optimized.as_bytes())?;
        changed += 1;
        saved += before.saturating_sub(after);
        println!(
            "  done    {}  {} -> {}",
            display_path(destination),
            format_bytes(before),
            format_bytes(after)
        );
    }
    print!(
        "justsvg: {changed} optimized, {} kept",
        plans.len() - changed
    );
    if saved > 0 {
        print!(", saved {}", format_bytes(saved));
    }
    println!();
    Ok(())
}

fn conservative_jobs(precision: u8) -> Result<Jobs> {
    // Start from OXVG's SVGO-compatible default preset, then apply the same
    // conservative overrides as the existing SVGOMG command.
    let mut jobs = Jobs::from_svgo_plugin_config(Some(vec![json!("preset-default")]))
        .map_err(|error| anyhow!("could not configure SVG optimizer: {error}"))?;
    jobs.cleanup_ids = None;
    jobs.remove_desc = None;
    jobs.remove_title = None;
    jobs.remove_view_box = None;
    jobs.remove_x_m_l_n_s = None;
    if let Some(job) = jobs.remove_unknowns_and_defaults.as_mut() {
        job.keep_aria_attrs = true;
        job.keep_role_attr = true;
    }
    if let Some(job) = jobs.cleanup_list_of_values.as_mut() {
        job.float_precision = precision;
    }
    if let Some(job) = jobs.cleanup_numeric_values.as_mut() {
        job.float_precision = precision;
    }
    if let Some(job) = jobs.convert_shape_to_path.as_mut() {
        job.float_precision = i32::from(precision);
    }
    if let Some(job) = jobs.convert_transform.as_mut() {
        job.float_precision = i32::from(precision);
    }
    if let Some(job) = jobs.convert_path_data.as_mut() {
        job.tolerance.precision = i32::from(precision);
    }
    Ok(jobs)
}

fn optimize_once(text: &str, source: &str, precision: u8) -> Result<String> {
    let jobs = conservative_jobs(precision)?;
    let nested = parse(text, |dom, allocator| -> Result<String> {
        jobs.run(dom, &Info::new(allocator))
            .map_err(|error| anyhow!("optimizer failed: {error}"))?;
        dom.serialize()
            .map_err(|error| anyhow!("could not serialize SVG: {error}"))
    })
    .map_err(|error| anyhow!("{source}: {error}"))?;
    nested.map_err(|error| anyhow!("{source}: {error:#}"))
}

fn optimize_svg(text: &str, source: &str, precision: u8, multipass: bool) -> Result<String> {
    let mut best = optimize_once(text, source, precision)?;
    if multipass {
        for _ in 1..10 {
            let candidate = optimize_once(&best, source, precision)?;
            if candidate.len() >= best.len() {
                break;
            }
            best = candidate;
        }
    }
    Ok(format!("{}\n", best.trim_end_matches(['\r', '\n'])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimizer_preserves_accessibility_and_ids() {
        let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" role="img" aria-labelledby="title"><title id="title">Test</title><path id="shape" d="M 0.0000 0.0000 L 10.0000 10.0000" /></svg>"#;
        let output = optimize_svg(input, "fixture.svg", 3, true).unwrap();
        assert!(output.contains("viewBox"));
        assert!(output.contains("role="));
        assert!(output.contains("aria-labelledby"));
        assert!(output.contains("<title"));
        assert!(output.contains("id=\"title\"") || output.contains("id='title'"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn malformed_svg_is_reported() {
        assert!(optimize_svg("<svg>", "bad.svg", 3, false).is_err());
    }
}
