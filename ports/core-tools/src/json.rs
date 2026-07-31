use crate::common::{
    absolute_lexical, atomic_write, collect_files, confirm, display_path, file_name, parse_cli,
    read_stdin, stdin_is_terminal, validate_unique_outputs,
};
use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "justjson",
    about = "Format, validate, query, or minify JSON.",
    after_help = "Files are formatted in place with two spaces and a final newline.\nPiped JSON is formatted to stdout. Object key order is preserved unless --sort is explicit."
)]
pub struct Cli {
    /// Validate only; never write files.
    #[arg(long)]
    check: bool,

    /// Print one value (for example: user.name or items[0]).
    #[arg(long, value_name = "PATH", conflicts_with_all = ["output", "dry_run", "check"])]
    get: Option<String>,

    /// Remove insignificant whitespace.
    #[arg(short = 'm', long, conflicts_with = "indent")]
    minify: bool,

    /// Sort object keys recursively.
    #[arg(short = 's', long)]
    sort: bool,

    /// Spaces per level, 0-8 (default: 2).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=8))]
    indent: Option<u8>,

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

    /// JSON files or folders. Defaults to the current folder on a terminal.
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
    let indent = options.indent.unwrap_or(2);
    if options.inputs.is_empty() && !stdin_is_terminal() {
        let text = read_stdin()?;
        if text.trim().is_empty() {
            bail!("no JSON received on stdin");
        }
        let mut value = parse_json(&text, "stdin")?;
        if options.sort {
            sort_value(&mut value);
        }
        if options.check {
            println!("justjson: valid JSON");
        } else if let Some(path) = options.get.as_deref() {
            print_selected(get_at_path(&value, path)?, options.minify, indent)?;
        } else {
            print!("{}", format_json(&value, options.minify, indent)?);
        }
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
    let collected = collect_files(
        &options.inputs,
        "json",
        options.recursive,
        output.as_deref(),
    )?;
    for warning in &collected.warnings {
        eprintln!("justjson: {warning}");
    }
    if collected.files.is_empty() {
        bail!("no JSON files found");
    }
    if options.get.is_some() && collected.files.len() != 1 {
        bail!("--get needs exactly one JSON file");
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
        println!("justjson: dry run — {} file(s)", plans.len());
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
        && !options.check
        && plans.iter().any(|(_, destination)| destination.exists())
        && !confirm(&format!(
            "justjson: format or replace {} file(s)",
            plans.len()
        ))?
    {
        bail!("cancelled");
    }

    let mut changed = 0usize;
    for (source, destination) in &plans {
        let original = fs::read_to_string(source)
            .with_context(|| format!("could not read {}", display_path(source)))?;
        let mut value = parse_json(&original, &display_path(source))?;
        if options.sort {
            sort_value(&mut value);
        }
        if let Some(path) = options.get.as_deref() {
            print_selected(get_at_path(&value, path)?, options.minify, indent)?;
            continue;
        }
        if options.check {
            continue;
        }
        let formatted = format_json(&value, options.minify, indent)?;
        if !destination.exists() || original.as_bytes() != formatted.as_bytes() {
            atomic_write(destination, formatted.as_bytes())?;
            changed += 1;
            println!("  done    {}", display_path(destination));
        } else {
            println!("  kept    {} (already formatted)", display_path(source));
        }
    }

    if options.check {
        println!("justjson: {} valid file(s)", plans.len());
    } else if options.get.is_none() {
        println!(
            "justjson: {changed} changed, {} kept",
            plans.len() - changed
        );
    }
    Ok(())
}

fn parse_json(text: &str, label: &str) -> Result<Value> {
    serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text))
        .map_err(|error| anyhow!("{label}: {error}"))
}

fn format_json(value: &Value, minify: bool, indent: u8) -> Result<String> {
    let mut output = Vec::new();
    if minify || indent == 0 {
        serde_json::to_writer(&mut output, value)?;
    } else {
        let spaces = vec![b' '; indent as usize];
        let formatter = serde_json::ser::PrettyFormatter::with_indent(&spaces);
        let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
        value.serialize(&mut serializer)?;
    }
    output.push(b'\n');
    String::from_utf8(output).context("JSON serializer produced invalid UTF-8")
}

fn sort_value(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(sort_value),
        Value::Object(map) => {
            for child in map.values_mut() {
                sort_value(child);
            }
            let old = std::mem::take(map);
            let mut entries: Vec<_> = old.into_iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            map.extend(entries);
        }
        _ => {}
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PathToken {
    Key(String),
    Index(usize),
}

fn path_tokens(expression: &str) -> Result<Vec<PathToken>> {
    let mut input = expression.trim();
    if input == "$" || input.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(rest) = input.strip_prefix('$') {
        input = rest;
    }
    if let Some(rest) = input.strip_prefix('.') {
        input = rest;
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let bytes = input.as_bytes();
    let mut offset = 0usize;
    let mut tokens = Vec::new();
    while offset < bytes.len() {
        if !tokens.is_empty() && !matches!(bytes[offset], b'.' | b'[') {
            bail!("invalid JSON path: {expression}");
        }
        if bytes[offset] == b'.' {
            offset += 1;
            if offset == bytes.len() {
                bail!("invalid JSON path: {expression}");
            }
        }
        if bytes[offset] == b'[' {
            offset += 1;
            if offset >= bytes.len() {
                bail!("invalid JSON path: {expression}");
            }
            if matches!(bytes[offset], b'\'' | b'\"') {
                let quote = bytes[offset];
                offset += 1;
                let mut key = String::new();
                let mut segment_start = offset;
                let mut closed = false;
                while offset < bytes.len() {
                    let byte = bytes[offset];
                    if byte == quote {
                        key.push_str(&input[segment_start..offset]);
                        offset += 1;
                        closed = true;
                        break;
                    }
                    if byte == b'\\' && offset + 1 < bytes.len() {
                        key.push_str(&input[segment_start..offset]);
                        let escaped = bytes[offset + 1];
                        if matches!(escaped, b'\\' | b'\'' | b'\"') {
                            key.push(escaped as char);
                            offset += 2;
                        } else {
                            key.push('\\');
                            offset += 1;
                        }
                        segment_start = offset;
                        continue;
                    }
                    offset += 1;
                }
                if !closed || bytes.get(offset) != Some(&b']') {
                    bail!("invalid JSON path: {expression}");
                }
                offset += 1;
                tokens.push(PathToken::Key(key));
            } else {
                let start = offset;
                while offset < bytes.len() && bytes[offset].is_ascii_digit() {
                    offset += 1;
                }
                if start == offset || bytes.get(offset) != Some(&b']') {
                    bail!("invalid JSON path: {expression}");
                }
                let index = input[start..offset]
                    .parse::<usize>()
                    .map_err(|_| anyhow!("invalid JSON path: {expression}"))?;
                offset += 1;
                tokens.push(PathToken::Index(index));
            }
        } else {
            let start = offset;
            while offset < bytes.len() && !matches!(bytes[offset], b'.' | b'[') {
                offset += 1;
            }
            if start == offset {
                bail!("invalid JSON path: {expression}");
            }
            tokens.push(PathToken::Key(input[start..offset].to_owned()));
        }
    }
    Ok(tokens)
}

fn get_at_path<'a>(value: &'a Value, expression: &str) -> Result<&'a Value> {
    let mut current = value;
    for token in path_tokens(expression)? {
        current = match token {
            PathToken::Key(key) => current.get(&key),
            PathToken::Index(index) => current.get(index),
        }
        .ok_or_else(|| anyhow!("JSON path not found: {expression}"))?;
    }
    Ok(current)
}

fn print_selected(value: &Value, minify: bool, indent: u8) -> Result<()> {
    if let Value::String(text) = value {
        println!("{text}");
    } else {
        print!("{}", format_json(value, minify, indent)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_path_forms() {
        assert_eq!(
            path_tokens("$.users[0]['display.name']").unwrap(),
            [
                PathToken::Key("users".into()),
                PathToken::Index(0),
                PathToken::Key("display.name".into())
            ]
        );
    }

    #[test]
    fn sorts_nested_objects_but_not_arrays() {
        let mut value: Value = serde_json::from_str(r#"{"z":{"b":1,"a":2},"a":[2,1]}"#).unwrap();
        sort_value(&mut value);
        assert_eq!(
            format_json(&value, true, 2).unwrap(),
            "{\"a\":[2,1],\"z\":{\"a\":2,\"b\":1}}\n"
        );
    }

    #[test]
    fn format_always_has_final_newline() {
        let value: Value = serde_json::from_str(r#"{"x":1}"#).unwrap();
        assert_eq!(format_json(&value, false, 2).unwrap(), "{\n  \"x\": 1\n}\n");
    }
}
